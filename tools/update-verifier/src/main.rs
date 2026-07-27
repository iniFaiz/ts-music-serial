use base64::{engine::general_purpose::STANDARD, Engine as _};
use minisign_verify::{PublicKey, Signature};
use std::{
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn signature_files(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            signature_files(&path, output)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("sig") {
            output.push(path);
        }
    }
    Ok(())
}

fn decode_text(value: &str, label: &str) -> Result<String, String> {
    let bytes = STANDARD
        .decode(value.trim())
        .map_err(|error| format!("Invalid base64 {label}: {error}"))?;
    String::from_utf8(bytes).map_err(|error| format!("Invalid UTF-8 {label}: {error}"))
}

fn verify_file(public_key: &PublicKey, signature_path: &Path) -> Result<(), String> {
    let signature_encoded =
        fs::read_to_string(signature_path).map_err(|error| error.to_string())?;
    let signature_text = decode_text(&signature_encoded, "signature")?;
    let signature = Signature::decode(&signature_text)
        .map_err(|error| format!("Invalid {}: {error}", signature_path.display()))?;

    let artifact_path = signature_path.with_extension("");
    if !artifact_path.is_file() {
        return Err(format!(
            "Signed artifact for {} does not exist",
            signature_path.display()
        ));
    }

    let mut verifier = public_key
        .verify_stream(&signature)
        .map_err(|error| format!("Cannot verify {}: {error}", artifact_path.display()))?;
    let mut artifact = fs::File::open(&artifact_path).map_err(|error| error.to_string())?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = artifact
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        verifier.update(&buffer[..count]);
    }
    verifier
        .finalize()
        .map_err(|error| format!("Bad signature for {}: {error}", artifact_path.display()))?;
    writeln!(std::io::stdout(), "verified {}", artifact_path.display())
        .map_err(|error| error.to_string())
}

fn run() -> Result<(), String> {
    let root = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| "Usage: update-verifier <bundle-directory>".to_string())?;
    if !root.is_dir() {
        return Err(format!(
            "Bundle directory does not exist: {}",
            root.display()
        ));
    }
    let public_key_encoded = env::var("TS_MUSIC_UPDATER_PUBLIC_KEY")
        .map_err(|_| "TS_MUSIC_UPDATER_PUBLIC_KEY is not set".to_string())?;
    let public_key_text = decode_text(&public_key_encoded, "public key")?;
    let public_key = PublicKey::decode(&public_key_text)
        .map_err(|error| format!("Invalid public key: {error}"))?;

    let mut signatures = Vec::new();
    signature_files(&root, &mut signatures)?;
    if signatures.is_empty() {
        return Err(format!("No updater signatures found in {}", root.display()));
    }
    signatures.sort();
    for signature in &signatures {
        verify_file(&public_key, signature)?;
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("update signature verification failed: {error}");
        std::process::exit(1);
    }
}
