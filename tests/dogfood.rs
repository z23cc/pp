mod common;

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
#[ignore = "expensive dogfood test: generates and builds fixture CLIs; run with `cargo test --test dogfood -- --ignored`"]
fn fixture_clis_expose_mcp_tools() {
    let fixtures = [
        (
            "plausible.yaml",
            "plausible-api",
            Some(("PLAUSIBLE_API_TOKEN", "dummy")),
        ),
        ("pokeapi.yaml", "poke-api", None),
        ("interzoid.yaml", "interzoid-get-weather-city-api", None),
    ];

    for (fixture, bin_name, env) in fixtures {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(fixture);
        let out_dir = temp.path().join("out");
        common::assert_success(
            common::run_pp_generate(&spec, &out_dir),
            &format!("pp generate --build {fixture}"),
        );
        let tools = list_tools(&common::generated_bin(&out_dir, bin_name), env);
        assert!(
            !tools.is_empty(),
            "{fixture} generated no MCP tools in tools/list"
        );
    }
}

fn list_tools(bin: &std::path::Path, env: Option<(&str, &str)>) -> Vec<Value> {
    let mut command = Command::new(bin);
    common::disable_proxy(&mut command);
    command
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if let Some((name, value)) = env {
        command.env(name, value);
    }
    let mut child = command.spawn().expect("spawn mcp");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));

    writeln!(
        stdin,
        "{}",
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}
        })
    )
    .unwrap();
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
    )
    .unwrap();
    drop(stdin);

    let mut line = String::new();
    let mut tools = Vec::new();
    while stdout.read_line(&mut line).expect("read mcp") > 0 {
        if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
            if value.get("id") == Some(&json!(2)) {
                tools = value["result"]["tools"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                break;
            }
        }
        line.clear();
    }
    let _ = child.kill();
    let _ = child.wait();
    tools
}
