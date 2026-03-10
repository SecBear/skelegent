//! hello-claude — minimal integration example.
//!
//! Reads the Anthropic OAuth token from OMP's agent.db,
//! sends a single inference request via SingleShotOperator,
//! and prints the response.

use layer0::content::Content;
use layer0::operator::{Operator, OperatorInput, TriggerType};
use skg_op_single_shot::{SingleShotConfig, SingleShotOperator};
use skg_provider_anthropic::AnthropicProvider;

/// Read the Anthropic OAuth access token from OMP's credential store.
///
/// OMP stores credentials in `~/.omp/agent/agent.db` (SQLite).
/// The `data` column is JSON: `{"access":"sk-ant-oat...","refresh":...,"expires":...}`.
fn read_omp_anthropic_token() -> Result<String, Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;
    let db_path = format!("{home}/.omp/agent/agent.db");

    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| format!("failed to open {db_path}: {e} — is OMP installed?"))?;

    let data: String = conn
        .query_row(
            "SELECT data FROM auth_credentials \
             WHERE provider='anthropic' AND credential_type='oauth' \
             ORDER BY updated_at DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("no anthropic oauth credential in agent.db: {e}"))?;

    let parsed: serde_json::Value = serde_json::from_str(&data)?;
    parsed["access"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "no 'access' field in credential data".into())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Read token from OMP's credential store.
    let token = read_omp_anthropic_token()?;
    eprintln!("token loaded ({} chars, ends ...{})", token.len(), &token[token.len().saturating_sub(4)..]);

    // 2. Build provider + operator.
    let provider = AnthropicProvider::new(token);
    let config = SingleShotConfig {
        system_prompt: "You are a helpful assistant. Be concise.".into(),
        default_model: "claude-sonnet-4-20250514".into(),
        default_max_tokens: 1024,
    };
    let op = SingleShotOperator::new(provider, config);

    // 3. Build input.
    let input = OperatorInput::new(
        Content::text("What is the meaning of the word 'skelegent'? Make one up if you don't know."),
        TriggerType::User,
    );

    // 4. Execute and print.
    let output = op.execute(input).await?;
    println!("Response: {}", output.message.as_text().unwrap_or("(no text)"));
    println!(
        "Tokens: in={}, out={}",
        output.metadata.tokens_in, output.metadata.tokens_out
    );
    println!("Cost: ${}", output.metadata.cost);

    Ok(())
}
