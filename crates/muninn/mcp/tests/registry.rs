mod common;
use muninn_mcp::{Catalog, Server, ToolHandler};
use serde_json::{json, Value};

struct Echo;
impl ToolHandler for Echo {
    fn call(&self, arguments: &Value) -> Result<Value, String> {
        Ok(arguments.clone())
    }
}

#[test]
fn catalogs_preserve_exact_fixtures() {
    for (catalog, fixture) in [
        (Catalog::Core, include_str!("../registry/core.json")),
        (Catalog::Extended, include_str!("../registry/extended.json")),
        (
            Catalog::ExtendedPlatinum,
            include_str!("../registry/extended-platinum.json"),
        ),
    ] {
        let expected: Value = serde_json::from_str(fixture).unwrap();
        assert_eq!(catalog.tools(), expected["tools"]);
    }
}

#[test]
fn only_registered_handlers_are_visible_and_callable() {
    let mut server = Server::default();
    assert!(server
        .register(Catalog::Core, "nonexistent", Box::new(Echo))
        .is_err());
    server
        .register(Catalog::Core, "memory_search", Box::new(Echo))
        .unwrap();
    assert!(server
        .register(Catalog::Core, "memory_search", Box::new(Echo))
        .is_err());
    let call = |server: &mut Server, method: &str, params: Value| {
        server.handle(common::request(method, params)).unwrap()
    };
    call(&mut server, "server/discover", json!({}));
    assert!(server
        .register(Catalog::Core, "memory_store_message", Box::new(Echo))
        .is_err());
    let listed = call(&mut server, "tools/list", json!({}));
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 1);
    assert_eq!(listed["result"]["tools"][0], Catalog::Core.tools()[0]);
    let response = call(
        &mut server,
        "tools/call",
        json!({"name":"memory_search","arguments":{"query":"hola"}}),
    );
    assert_eq!(response["result"]["isError"], false);
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(text).unwrap(),
        json!({"query":"hola"})
    );
    assert_eq!(
        call(
            &mut server,
            "tools/call",
            json!({"name":"memory_search","arguments":[]})
        )["error"]["code"],
        -32602
    );
}
