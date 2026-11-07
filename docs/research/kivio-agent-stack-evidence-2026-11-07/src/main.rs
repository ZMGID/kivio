use serde_json::{json, Map, Value};

fn main() {
    let count: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    let phase = std::env::args().nth(2).unwrap_or_else(|| "coerce".into());
    std::thread::Builder::new()
        .name("schema-probe-2mib".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let mut definitions = Map::new();
            for index in 0..count {
                let definition = if index + 1 == count {
                    json!({"type": "integer"})
                } else {
                    json!({"$ref": format!("#/$defs/d{}", index + 1)})
                };
                definitions.insert(format!("d{index}"), definition);
            }
            let schema = json!({"$ref": "#/$defs/d0", "$defs": definitions});
            eprintln!("ready count={count} phase={phase} schema_bytes={}", serde_json::to_vec(&schema).unwrap().len());
            if phase == "build_map" {
                let validators = jsonschema::options().build_map(&schema).unwrap();
                assert!(validators.get("#").is_some());
                eprintln!("compiled_map count={count}");
            } else if phase == "validate" {
                kivio_agent_sdk::tools::schema::validate_arguments(&schema, &json!(7)).unwrap();
                eprintln!("validated count={count}");
            } else {
                let output = kivio_agent_sdk::tools::schema::coerce_arguments(&schema, Value::String("7".into()));
                assert_eq!(output, json!(7));
                eprintln!("coerced count={count}");
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
