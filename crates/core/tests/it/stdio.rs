use super::*;
use codemode_core::{
    components::sources::{Config, Source, Transport},
    infrastructure::sandbox::MontySandbox,
};

#[sqlx::test(migrations = "./migrations")]
async fn stdio_dispatch_only_receives_the_explicit_environment(pool: sqlx::PgPool) {
    let stop = CancellationToken::new();
    let tasks = TaskTracker::new();
    let db = database(&pool).await;
    let auth = Auth::new(db.clone());
    let (_, token) = auth.create("stdio", "test.example").await.unwrap();
    let principal = auth
        .authenticate(&token, "test.example")
        .await
        .unwrap()
        .unwrap();
    let catalogue = Catalogue::new(db.clone());
    let sources = managed_sources(db.clone(), catalogue);
    let sync = sources.clone();
    let sync_stop = stop.clone();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = std::env::var_os("CODEMODE_STDIO_FIXTURE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("tests/fixtures/stdio.py"));
    let config = Config {
        sources: vec![Source {
            requires_account: false,
            slug: "fixture".into(),
            transport: Transport::McpStdio {
                command: "/usr/bin/python3".into(),
                args: vec![fixture.to_str().unwrap().into()],
                env: [("FIXTURE_SECRET".into(), "HOME".into())].into(),
            },
        }],
    };
    config.validate().unwrap();
    sync.import(config).await.unwrap();
    tasks.spawn(async move {
        sync.run(sync_stop).await;
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while sources.lookup("fixture.probe").await.is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stdio fixture should synchronize");
    let worker = std::env::var_os("CODEMODE_MONTY_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("../../.tools/monty/bin/monty"));
    let sandbox = Arc::new(
        MontySandbox::new(&worker, std::time::Duration::from_secs(1), 32768)
            .await
            .unwrap(),
    );
    let checkout = sandbox.checkout().await.unwrap();
    assert!(
        std::fs::read(format!("/proc/{}/environ", checkout.pid().unwrap()))
            .unwrap()
            .is_empty()
    );
    drop(checkout);
    let execution = Arc::new(Execution::new(
        db,
        sources,
        sandbox.clone(),
        tasks.clone(),
        stop.clone(),
        Default::default(),
    ));
    let result = execution
        .execute(
            principal,
            r#"return await tools.fixture.probe(value="hello")"#.into(),
        )
        .await
        .unwrap();
    assert_eq!(result.status, "completed", "{:?}", result.error);
    let value: serde_json::Value = serde_json::from_str(result.result.get()).unwrap();
    assert_eq!(value["value"], "hello");
    assert_eq!(value["linked"], true);
    let environment = value["environment"].as_array().unwrap();
    assert!(
        environment
            .iter()
            .all(|name| name == "FIXTURE_SECRET" || name == "LC_CTYPE")
    );
    assert!(environment.iter().any(|name| name == "FIXTURE_SECRET"));
    stop.cancel();
    tasks.close();
    tasks.wait().await;
    sandbox.pool.close().await;
}
