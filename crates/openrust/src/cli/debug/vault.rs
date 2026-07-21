use clap::Subcommand;

use crate::core::crypto;

#[derive(Subcommand)]
pub enum Cmd {
    /// Test encrypt/decrypt round-trip
    Test {
        /// Plaintext to encrypt then decrypt
        #[arg(short, long, default_value = "test-key-42")]
        text: String,
    },
    /// Show which providers have encrypted keys
    Status,
    /// Encrypt a provider's API key (reads from stdin if --api-key not provided)
    Set {
        provider: String,
        #[arg(long)]
        api_key: Option<String>,
    },
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    match cmd {
        Cmd::Test { text } => {
            println!("Plaintext:  {}", text);
            let encrypted = crypto::encrypt_api_key(&text)
                .ok_or_else(|| anyhow::anyhow!("Encryption failed — no machine-id?"))?;
            println!(
                "Encrypted:  {}... ({} bytes)",
                &encrypted[..32.min(encrypted.len())],
                encrypted.len()
            );
            let decrypted = crypto::decrypt_api_key(&encrypted)
                .ok_or_else(|| anyhow::anyhow!("Decryption failed!"))?;
            println!("Decrypted:  {}", decrypted);
            if decrypted != text {
                anyhow::bail!("Round-trip mismatch!");
            }
            println!("✅ Round-trip OK");
            Ok(())
        }
        Cmd::Status => {
            let vault = crate::core::vault::Vault::load();
            let mut found = false;
            // We can't enumerate keys from Vault (private), but we can check
            // via get_provider for each provider in config
            let cwd = std::env::current_dir()?;
            let config = crate::core::config::Config::load(&cwd)?;
            for name in config.provider.keys() {
                match vault.get(name) {
                    Some(k) => {
                        println!("🔐 {}: {}", name, super::mask_secret(&k));
                        found = true;
                    }
                    None => {
                        println!("  {}: (no vault entry)", name);
                    }
                }
            }
            if !found && config.provider.is_empty() {
                println!(
                    "No providers configured. Set one first: openrust debug config set <name> --base-url <url>"
                );
            }
            Ok(())
        }
        Cmd::Set { provider, api_key } => {
            let key = match api_key {
                Some(k) => k,
                None => {
                    println!("Enter API key for '{}':", provider);
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input)?;
                    input.trim().to_string()
                }
            };
            if key.is_empty() {
                anyhow::bail!("API key cannot be empty");
            }
            crate::core::vault::Vault::save(&provider, &key)?;
            println!("🔐 Saved encrypted API key for '{}'.", provider);
            Ok(())
        }
    }
}
