use clap::Parser;

#[derive(Debug, serde::Serialize)]
struct JsonOutput {
    ok: bool,
    tool: &'static str,
}

#[derive(Debug, Parser)]
#[command(name = "prose", version, about)]
struct Cli {
    /// Print output as JSON.
    #[arg(long)]
    json: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if cli.json {
        let output = JsonOutput {
            ok: true,
            tool: "prose",
        };
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("prose: hello from prose");
    }
    Ok(())
}
