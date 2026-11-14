use std::{path::PathBuf, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn kernel_denies_network_host_files_process_creation_and_enforces_worker_cgroup() {
    let entry = PathBuf::from(
        std::env::var_os("CODEMODE_MONTY_BIN").expect("run with T_ISOLATED=1 on Oracle"),
    );
    assert_eq!(
        entry.file_name().unwrap(),
        "worker.sh",
        "this test requires the isolated entrypoint"
    );
    let dir = Directory(
        std::env::temp_dir().join(format!("codemode-isolation-{}", uuid::Uuid::new_v4())),
    );
    tokio::fs::create_dir(&dir.0).await.unwrap();
    tokio::fs::copy(&entry, dir.0.join("worker.sh"))
        .await
        .unwrap();
    tokio::fs::copy(
        entry.with_file_name("worker-guard.py"),
        dir.0.join("worker-guard.py"),
    )
    .await
    .unwrap();
    tokio::fs::copy(entry.with_file_name("worker.bpf"), dir.0.join("worker.bpf"))
        .await
        .unwrap();
    tokio::fs::write(dir.0.join("probe.c"), include_str!("probe.c"))
        .await
        .unwrap();
    let built = Command::new("/usr/bin/cc")
        .args(["-O2", "-pthread", "-o"])
        .arg(dir.0.join("monty"))
        .arg(dir.0.join("probe.c"))
        .output()
        .await
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let mut child = Command::new(dir.0.join("worker.sh"))
        .arg("subprocess")
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .await
        .unwrap();
    if line.is_empty() {
        let output = child.wait_with_output().await.unwrap();
        panic!(
            "isolation probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let result: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        result,
        serde_json::json!({"environmentEmpty":true,"privatePid":2,"noNewPrivileges":true,"seccomp":2,"networkDenied":true,"hostFilesDenied":true,"writesDenied":true,"forkDenied":true,"threadsWork":true})
    );
    assert!(
        tokio::fs::read(format!("/proc/{pid}/environ"))
            .await
            .unwrap()
            .is_empty()
    );
    let membership = tokio::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .await
        .unwrap();
    let group = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .unwrap();
    assert!(group.ends_with(".scope"), "{group}");
    let cgroup = PathBuf::from("/sys/fs/cgroup").join(group.trim_start_matches('/'));
    for (file, expected) in [
        ("memory.max", "67108864"),
        ("memory.swap.max", "0"),
        ("pids.max", "16"),
        ("memory.oom.group", "1"),
    ] {
        assert_eq!(
            tokio::fs::read_to_string(cgroup.join(file))
                .await
                .unwrap()
                .trim(),
            expected
        );
    }
    child.stdin.take().unwrap().write_all(b"\n").await.unwrap();
    assert!(child.wait().await.unwrap().success());
    for _ in 0..8 {
        let mut child = Command::new(dir.0.join("worker.sh"))
            .arg("subprocess")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let cgroup = worker_group(pid).await;
        child.start_kill().unwrap();
        child.wait().await.unwrap();
        assert_empty_group(&cgroup).await;
    }
    let mut owner = Command::new("/usr/bin/python3")
        .args(["-I", "-S", "-c", "import subprocess,sys; p=subprocess.Popen([sys.argv[1],'subprocess'],env={},stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); print(p.pid,flush=True); sys.stdin.buffer.read(1)"])
        .arg(dir.0.join("worker.sh")).env_clear().stdin(Stdio::piped()).stdout(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
    let mut line = String::new();
    BufReader::new(owner.stdout.take().unwrap())
        .read_line(&mut line)
        .await
        .unwrap();
    let cgroup = worker_group(line.trim().parse().unwrap()).await;
    // Keep the inherited stdin writer alive: cleanup must follow owner death, not protocol EOF.
    let input = owner.stdin.take().unwrap();
    owner.start_kill().unwrap();
    owner.wait().await.unwrap();
    assert_empty_group(&cgroup).await;
    drop(input);
    println!(
        "PASS: actual isolated process, empty environment, namespaces, seccomp denials and 64 MiB / zero swap / 16-task cgroup"
    );
}

async fn worker_group(pid: u32) -> PathBuf {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let membership = tokio::fs::read_to_string(format!("/proc/{pid}/cgroup"))
                .await
                .unwrap();
            let group = membership
                .lines()
                .find_map(|line| line.strip_prefix("0::"))
                .unwrap();
            let path = PathBuf::from("/sys/fs/cgroup").join(group.trim_start_matches('/'));
            if path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("run-")
                && tokio::fs::read_to_string(path.join("cgroup.procs"))
                    .await
                    .unwrap()
                    .lines()
                    .count()
                    >= 3
            {
                break path;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

async fn assert_empty_group(cgroup: &std::path::Path) {
    tokio::time::timeout(std::time::Duration::from_millis(500), async {
        loop {
            match tokio::fs::read_to_string(cgroup.join("cgroup.events")).await {
                Ok(events) if !events.lines().any(|line| line == "populated 1") => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                _ => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .expect("pool launcher death must empty its complete cgroup");
}
