use codemode_core::components::catalogue::stubs::{generate, names};

fn main() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../../../docs/trial-schemas.json")).unwrap();
    for (source, tools) in corpus.as_object().unwrap() {
        let tools = tools.as_array().unwrap();
        let aliases = names(tools.iter().map(|tool| tool["name"].as_str().unwrap()));
        for tool in tools {
            let name = tool["name"].as_str().unwrap();
            let stub = generate(
                &format!("{source}.{}", aliases[name]),
                tool["description"].as_str().unwrap_or_default(),
                &tool["inputSchema"],
                tool.get("outputSchema"),
            );
            print!("## {source}.{name}\n{}\n{}\n\n", stub.brief, stub.full);
        }
    }
}
