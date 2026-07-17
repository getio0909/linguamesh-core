use clap::{Parser, Subcommand};
use linguamesh_domain::{TranslationEvent, TranslationRequest};
use linguamesh_engine::TranslationEngine;
use linguamesh_provider_openai::{OpenAiCompatibleProvider, OpenAiConfig};
use linguamesh_storage::Storage;
use linguamesh_testkit::FakeProviderServer;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "linguamesh", about = "LinguaMesh reference CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Run a real streamed translation against a loopback fake provider")]
    Demo {
        #[arg(long, help = "Source text to translate")]
        text: String,
        #[arg(long, help = "BCP 47 target language tag")]
        target: String,
        #[arg(
            long,
            default_value = "fake-translator",
            help = "Fake provider model identifier"
        )]
        model: String,
        #[arg(
            long,
            help = "Cancel the request after the given delay in milliseconds"
        )]
        cancel_after_ms: Option<u64>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Demo {
            text,
            target,
            model,
            cancel_after_ms,
        } => run_demo(text, target, model, cancel_after_ms).await?,
    }
    Ok(())
}

async fn run_demo(
    text: String,
    target: String,
    model: String,
    cancel_after_ms: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let server = FakeProviderServer::start().await?;
    let provider =
        OpenAiCompatibleProvider::new(OpenAiConfig::without_credential(server.base_url()))?;
    let engine = TranslationEngine::new(Arc::new(provider));
    let discovered = engine.list_models().await?;
    println!(
        "Discovered models: {}",
        discovered
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let storage = Storage::in_memory()?;
    storage.upsert_manual_model(&model)?;
    storage.set_active_model(&model)?;
    println!("Selected model: {model}");
    print!("Streamed translation: ");
    io::stdout().flush()?;

    let mut operation = engine.translate(TranslationRequest::new(text, target, model));
    let wait = cancel_after_ms.map_or(Duration::MAX, Duration::from_millis);
    let cancellation_timer = tokio::time::sleep(wait);
    tokio::pin!(cancellation_timer);
    let mut cancellation_requested = false;
    let mut terminal = None;
    loop {
        tokio::select! {
            () = &mut cancellation_timer, if !cancellation_requested => {
                operation.cancel();
                cancellation_requested = true;
            }
            event = operation.next_event() => {
                let Some(event) = event else {
                    break;
                };
                match event {
                    TranslationEvent::TextDelta { text, .. } => {
                        print!("{text}");
                        io::stdout().flush()?;
                    }
                    event if event.is_terminal() => {
                        terminal = Some(event);
                    }
                    _ => {}
                }
            }
        }
    }
    println!();
    match terminal {
        Some(TranslationEvent::Completed { .. }) => println!("Translation completed."),
        Some(TranslationEvent::Cancelled { .. }) => println!("Translation cancelled."),
        Some(TranslationEvent::Failed { error, .. }) => return Err(Box::new(error)),
        _ => return Err("Translation ended without a terminal event.".into()),
    }
    server.shutdown().await;
    Ok(())
}
