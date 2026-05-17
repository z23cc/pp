use crate::context::Context;
use crate::invoke::{invoke_operation, ArgDef, OperationInvocation};
use anyhow::{anyhow, bail, Result};
use rmcp::model::JsonObject;
use serde_json::{Number, Value};

#[derive(Clone, Copy)]
enum CliPrimitiveKind {
    String,
    Integer,
    Number,
    Boolean,
}

#[derive(Clone, Copy)]
enum CliValueKind {
    String,
    Integer,
    Number,
    Boolean,
    PrimitiveArray { item: CliPrimitiveKind },
    NullablePrimitive { item: CliPrimitiveKind },
    Json,
}

struct CliArgDef {
    json_name: &'static str,
    required: bool,
    value_kind: CliValueKind,
}

struct CliOperationDef {
    name: &'static str,
    description: &'static str,
    method: &'static str,
    path_template: &'static str,
    args: &'static [ArgDef],
    cli_args: &'static [CliArgDef],
}


static ARGS_1: &[ArgDef] = &[

    ArgDef { json_name: "count", binding: crate::invoke::ArgBinding::FlattenedJsonBodyField },

    ArgDef { json_name: "enabled", binding: crate::invoke::ArgBinding::FlattenedJsonBodyField },

    ArgDef { json_name: "name", binding: crate::invoke::ArgBinding::FlattenedJsonBodyField },

];

static CLI_ARGS_1: &[CliArgDef] = &[

    CliArgDef { json_name: "count", required: false, value_kind: CliValueKind::Integer },

    CliArgDef { json_name: "enabled", required: false, value_kind: CliValueKind::Boolean },

    CliArgDef { json_name: "name", required: true, value_kind: CliValueKind::String },

];

static ARGS_2: &[ArgDef] = &[

    ArgDef { json_name: "itemId", binding: crate::invoke::ArgBinding::PathParam { wire_name: "itemId" } },

    ArgDef { json_name: "include_details", binding: crate::invoke::ArgBinding::QueryParam { wire_name: "include_details" } },

    ArgDef { json_name: "tag", binding: crate::invoke::ArgBinding::QueryParam { wire_name: "tag" } },

];

static CLI_ARGS_2: &[CliArgDef] = &[

    CliArgDef { json_name: "itemId", required: true, value_kind: CliValueKind::String },

    CliArgDef { json_name: "include_details", required: false, value_kind: CliValueKind::Boolean },

    CliArgDef { json_name: "tag", required: false, value_kind: CliValueKind::PrimitiveArray { item: CliPrimitiveKind::String } },

];


static OPERATIONS: &[CliOperationDef] = &[

    CliOperationDef {
        name: "create_item",
        description: "Create one item [auth: NATIVE_CORE_API_TOKEN env var]",
        method: "POST",
        path_template: "/items",
        args: ARGS_1,
        cli_args: CLI_ARGS_1,
    },

    CliOperationDef {
        name: "get_item",
        description: "Fetch one item [auth: NATIVE_CORE_API_TOKEN env var]",
        method: "GET",
        path_template: "/items/{itemId}",
        args: ARGS_2,
        cli_args: CLI_ARGS_2,
    },

];

pub async fn run() -> Result<()> {
    let matches = build_command().get_matches();
    if matches.subcommand_name() == Some("mcp") {
        let context = Context::new_mcp()?;
        return crate::mcp::serve(context).await;
    }

    let context = if matches.get_flag("json") {
        match Context::new_json() {
            Ok(context) => context,
            Err(error) => {
                println!(
                    "{}",
                    serde_json::json!({
                        "error": {
                            "kind": "context",
                            "status": null,
                            "body": error.to_string(),
                            "headers": {},
                        }
                    })
                );
                return Err(error);
            }
        }
    } else {
        Context::new()?
    };
    run_with_matches(context, &matches).await
}

pub async fn run_with_matches(context: Context, matches: &clap::ArgMatches) -> Result<()> {
    if let Some((name, matches)) = matches.subcommand() {
        if name == "mcp" {
            unreachable!("mcp is handled before generated dispatch")
        }
        let operation = OPERATIONS
            .iter()
            .find(|operation| operation.name == name)
            .ok_or_else(|| anyhow!("clap returned an unknown generated command: {name}"))?;
        let arguments = arguments_from_matches(operation, matches)?;
        let result = invoke_operation(
            context.clone(),
            OperationInvocation {
                name: operation.name,
                method: operation.method,
                path_template: operation.path_template,
                args: operation.args,
                arguments,
            },
        )
        .await
        .map_err(|error| anyhow!(error.to_string()))?;

        if result.is_error {
            crate::print::emit_cli_error(&context, result.value)?;
            bail!("API request failed");
        }
        crate::print::emit_cli_success(&context, result)
    } else {
        build_command().print_help()?;
        println!();
        Ok(())
    }
}

fn build_command() -> clap::Command {
    let mut command = clap::Command::new("native-core-api")
        .about("Generated API CLI")
        .arg(
            clap::Arg::new("json")
                .long("json")
                .help("Print one structured JSON value to stdout")
                .global(true)
                .action(clap::ArgAction::SetTrue),
        )
        .subcommand(clap::Command::new("mcp").about("Run an MCP server over stdio"));
    for operation in OPERATIONS {
        let mut subcommand = clap::Command::new(operation.name).about(operation.description);
        for arg in operation.cli_args {
            let action = if matches!(arg.value_kind, CliValueKind::PrimitiveArray { .. }) {
                clap::ArgAction::Append
            } else {
                clap::ArgAction::Set
            };
            subcommand = subcommand.arg(
                clap::Arg::new(arg.json_name)
                    .long(arg.json_name)
                    .required(arg.required)
                    .num_args(1)
                    .action(action),
            );
        }
        command = command.subcommand(subcommand);
    }
    command
}

fn arguments_from_matches(
    operation: &CliOperationDef,
    matches: &clap::ArgMatches,
) -> Result<JsonObject> {
    let mut arguments = JsonObject::new();
    for arg in operation.cli_args {
        match arg.value_kind {
            CliValueKind::PrimitiveArray { item } => {
                if let Some(values) = matches.get_many::<String>(arg.json_name) {
                    let array = values
                        .map(|value| parse_primitive(value, item, arg.json_name))
                        .collect::<Result<Vec<_>>>()?;
                    if !array.is_empty() {
                        arguments.insert(arg.json_name.to_string(), Value::Array(array));
                    }
                }
            }
            value_kind => {
                if let Some(value) = matches.get_one::<String>(arg.json_name) {
                    arguments.insert(arg.json_name.to_string(), parse_cli_value(value, value_kind, arg.json_name)?);
                }
            }
        }
    }
    Ok(arguments)
}

fn parse_cli_value(raw: &str, value_kind: CliValueKind, arg_name: &str) -> Result<Value> {
    match value_kind {
        CliValueKind::String => Ok(Value::String(raw.to_string())),
        CliValueKind::Integer => parse_integer(raw, arg_name),
        CliValueKind::Number => parse_number(raw, arg_name),
        CliValueKind::Boolean => parse_boolean(raw, arg_name),
        CliValueKind::Json => serde_json::from_str(raw)
            .with_context(|| format!("--{arg_name} must be valid JSON")),
        CliValueKind::NullablePrimitive { item } => {
            if raw == "null" {
                Ok(Value::Null)
            } else {
                parse_primitive(raw, item, arg_name)
            }
        }
        CliValueKind::PrimitiveArray { item } => parse_primitive(raw, item, arg_name),
    }
}

fn parse_primitive(raw: &str, kind: CliPrimitiveKind, arg_name: &str) -> Result<Value> {
    match kind {
        CliPrimitiveKind::String => Ok(Value::String(raw.to_string())),
        CliPrimitiveKind::Integer => parse_integer(raw, arg_name),
        CliPrimitiveKind::Number => parse_number(raw, arg_name),
        CliPrimitiveKind::Boolean => parse_boolean(raw, arg_name),
    }
}

fn parse_integer(raw: &str, arg_name: &str) -> Result<Value> {
    let value = raw
        .parse::<i64>()
        .with_context(|| format!("--{arg_name} must be an integer"))?;
    Ok(Value::Number(Number::from(value)))
}

fn parse_number(raw: &str, arg_name: &str) -> Result<Value> {
    let value = raw
        .parse::<f64>()
        .with_context(|| format!("--{arg_name} must be a number"))?;
    let number = Number::from_f64(value).ok_or_else(|| anyhow!("--{arg_name} must be a finite number"))?;
    Ok(Value::Number(number))
}

fn parse_boolean(raw: &str, arg_name: &str) -> Result<Value> {
    match raw {
        "true" | "1" => Ok(Value::Bool(true)),
        "false" | "0" => Ok(Value::Bool(false)),
        _ => bail!("--{arg_name} must be a boolean: true, false, 1, or 0"),
    }
}

use anyhow::Context as _;