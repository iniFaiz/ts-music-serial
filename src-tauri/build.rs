use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use syn::{FnArg, GenericArgument, Item, Pat, PathArguments, ReturnType, Type};
use walkdir::WalkDir;

#[derive(Clone)]
struct Command {
    name: String,
    args: Vec<Argument>,
    result: String,
}

#[derive(Clone)]
struct Argument {
    name: String,
    ty: String,
    optional: bool,
}

fn main() {
    println!("cargo:rerun-if-changed=src");
    if std::env::var("TARGET").is_ok_and(|target| target.contains("windows-msvc")) {
        // Unit-test executables do not inherit Tauri's application manifest.
        // `windows` imports TaskDialogIndirect from Common-Controls v6; without
        // this activation-context dependency Windows loads the v5 system DLL
        // and aborts before libtest starts with STATUS_ENTRYPOINT_NOT_FOUND.
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
    let commands = discover_commands(Path::new("src"));
    assert!(!commands.is_empty(), "no #[tauri::command] functions found");

    let command_names: &'static [&'static str] = Box::leak(
        commands
            .keys()
            .map(|name| Box::leak(name.clone().into_boxed_str()) as &'static str)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(command_names)),
    )
    .expect("failed to build Tauri application manifest");

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set"));
    let names = commands.keys().cloned().collect::<Vec<_>>().join("\n");
    fs::write(out_dir.join("ipc-command-names.txt"), names)
        .expect("failed to write IPC command registry");

    let target = Path::new("../src/generated/ipc.ts");
    let generated = render_typescript(commands.values());
    if fs::read_to_string(target).ok().as_deref() != Some(generated.as_str()) {
        fs::create_dir_all(target.parent().expect("generated directory has a parent"))
            .expect("failed to create generated IPC directory");
        fs::write(target, generated).expect("failed to write generated IPC client");
    }
}

fn discover_commands(root: &Path) -> BTreeMap<String, Command> {
    let mut commands = BTreeMap::new();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|v| v.to_str()) == Some("rs"))
    {
        let source = fs::read_to_string(entry.path())
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", entry.path().display()));
        let file = syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("failed to parse {}: {error}", entry.path().display()));
        collect_items(&file.items, &mut commands);
    }
    commands
}

fn collect_items(items: &[Item], commands: &mut BTreeMap<String, Command>) {
    for item in items {
        match item {
            Item::Fn(function) if is_tauri_command(&function.attrs) => {
                let name = function.sig.ident.to_string();
                let args = function
                    .sig
                    .inputs
                    .iter()
                    .filter_map(|input| match input {
                        FnArg::Receiver(_) => None,
                        FnArg::Typed(argument) if is_injected(&argument.ty) => None,
                        FnArg::Typed(argument) => {
                            let Pat::Ident(pattern) = argument.pat.as_ref() else {
                                return None;
                            };
                            Some(Argument {
                                name: camel_case(pattern.ident.to_string().trim_start_matches('_')),
                                ty: ts_type(&argument.ty),
                                optional: is_option(&argument.ty),
                            })
                        }
                    })
                    .collect();
                let result = match &function.sig.output {
                    ReturnType::Default => "void".to_string(),
                    ReturnType::Type(_, ty) => ts_type(ty),
                };
                let command = Command {
                    name: name.clone(),
                    args,
                    result,
                };
                if let Some(existing) = commands.insert(name.clone(), command.clone()) {
                    assert_eq!(
                        render_signature(&existing),
                        render_signature(&command),
                        "cfg variants of command {name} have different IPC signatures"
                    );
                }
            }
            Item::Mod(module) => {
                if let Some((_, nested)) = &module.content {
                    collect_items(nested, commands);
                }
            }
            _ => {}
        }
    }
}

fn is_tauri_command(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        let mut segments = attribute.path().segments.iter().rev();
        segments
            .next()
            .is_some_and(|segment| segment.ident == "command")
            && segments
                .next()
                .is_some_and(|segment| segment.ident == "tauri")
    })
}

fn is_injected(ty: &Type) -> bool {
    let Type::Path(path) = peel_reference(ty) else {
        return false;
    };
    matches!(
        path.path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .as_deref(),
        Some("AppHandle" | "State" | "Window" | "Webview" | "WebviewWindow")
    )
}

fn is_option(ty: &Type) -> bool {
    let Type::Path(path) = peel_reference(ty) else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Option")
}

fn peel_reference(mut ty: &Type) -> &Type {
    while let Type::Reference(reference) = ty {
        ty = &reference.elem;
    }
    ty
}

fn ts_type(ty: &Type) -> String {
    match peel_reference(ty) {
        Type::Tuple(tuple) if tuple.elems.is_empty() => "void".to_string(),
        Type::Tuple(tuple) => format!(
            "[{}]",
            tuple
                .elems
                .iter()
                .map(ts_type)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Type::Array(array) => format!("{}[]", ts_type(&array.elem)),
        Type::Slice(slice) => format!("{}[]", ts_type(&slice.elem)),
        Type::Path(path) => {
            let Some(segment) = path.path.segments.last() else {
                return "unknown".to_string();
            };
            let name = segment.ident.to_string();
            let generics = match &segment.arguments {
                PathArguments::AngleBracketed(arguments) => arguments
                    .args
                    .iter()
                    .filter_map(|argument| match argument {
                        GenericArgument::Type(ty) => Some(ty),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            match name.as_str() {
                "Result" => generics
                    .first()
                    .map_or_else(|| "unknown".to_string(), |ty| ts_type(ty)),
                "Option" => generics.first().map_or_else(
                    || "unknown | null".to_string(),
                    |ty| format!("{} | null", ts_type(ty)),
                ),
                "Vec" | "VecDeque" | "HashSet" | "BTreeSet" => generics.first().map_or_else(
                    || "unknown[]".to_string(),
                    |ty| format!("{}[]", ts_type(ty)),
                ),
                "HashMap" | "BTreeMap" => generics.get(1).map_or_else(
                    || "Record<string, unknown>".to_string(),
                    |ty| format!("Record<string, {}>", ts_type(ty)),
                ),
                "bool" => "boolean".to_string(),
                "String" | "str" | "Path" | "PathBuf" => "string".to_string(),
                "f32" | "f64" | "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16"
                | "u32" | "u64" | "u128" | "usize" => "number".to_string(),
                "Value" => "unknown".to_string(),
                // Custom serde types are still called out in the generated
                // contract, without pretending their shape is known here.
                _ => format!("unknown /* Rust: {name} */"),
            }
        }
        _ => "unknown".to_string(),
    }
}

fn render_typescript<'a>(commands: impl Iterator<Item = &'a Command>) -> String {
    let commands = commands.collect::<Vec<_>>();
    let mut output = String::from(
        "// @generated by src-tauri/build.rs from #[tauri::command] Rust signatures.\n\
         // Do not edit by hand; run `cargo check --manifest-path src-tauri/Cargo.toml`.\n\
         import { invoke } from '@tauri-apps/api/core';\n\n",
    );
    output.push_str("export const COMMAND_NAMES = [\n");
    for command in &commands {
        output.push_str(&format!("  '{}',\n", command.name));
    }
    output.push_str("] as const;\n\nexport type CommandName = (typeof COMMAND_NAMES)[number];\n\n");
    output.push_str("export interface CommandArgs {\n");
    for command in &commands {
        let args = if command.args.is_empty() {
            "undefined".to_string()
        } else {
            format!(
                "{{ {} }}",
                command
                    .args
                    .iter()
                    .map(|argument| format!(
                        "{}{}: {}",
                        argument.name,
                        if argument.optional { "?" } else { "" },
                        argument.ty
                    ))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        };
        output.push_str(&format!("  '{}': {};\n", command.name, args));
    }
    output.push_str("}\n\nexport interface CommandResult {\n");
    for command in &commands {
        output.push_str(&format!("  '{}': {};\n", command.name, command.result));
    }
    output.push_str(
        "}\n\nexport function invokeCommand<K extends CommandName>(\n\
           command: K,\n\
           args: CommandArgs[K],\n\
         ): Promise<CommandResult[K]> {\n\
           return invoke<CommandResult[K]>(command, args);\n\
         }\n\n",
    );
    output.push_str("export const ipc = {\n");
    for command in &commands {
        let method = camel_case(&command.name);
        if command.args.is_empty() {
            output.push_str(&format!(
                "  {method}: () => invokeCommand('{}', undefined),\n",
                command.name
            ));
        } else {
            output.push_str(&format!(
                "  {method}: (args: CommandArgs['{}']) => invokeCommand('{}', args),\n",
                command.name, command.name
            ));
        }
    }
    output.push_str("} as const;\n");
    output
}

fn render_signature(command: &Command) -> String {
    format!(
        "{}({}):{}",
        command.name,
        command
            .args
            .iter()
            .map(|argument| format!("{}:{}", argument.name, argument.ty))
            .collect::<Vec<_>>()
            .join(","),
        command.result
    )
}

fn camel_case(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut uppercase = false;
    for character in value.chars() {
        if character == '_' {
            uppercase = true;
        } else if uppercase {
            result.extend(character.to_uppercase());
            uppercase = false;
        } else {
            result.push(character);
        }
    }
    result
}
