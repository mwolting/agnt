use agnt_llm::stream::StreamEvent;
use agnt_llm_openai::{OpenAIRequestExt, ReasoningEffort};
use std::io::Write;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    let prompt: String = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    if prompt.is_empty() {
        eprintln!("usage: agnt <prompt>");
        std::process::exit(1);
    }

    let provider = agnt_llm_openai::from_env();
    let model = provider.model("gpt-5-nano");

    let mut req = agnt_llm::request();
    req.reasoning_effort(ReasoningEffort::Minimal)
        .user(&prompt);

    let mut stream = model.generate(req).events();

    while let Some(event) = stream.next().await {
        match event {
            Ok(StreamEvent::TextDelta(delta)) => {
                print!("{delta}");
                let _ = std::io::stdout().flush();
            }
            Ok(StreamEvent::Finish { .. }) => {
                println!();
            }
            Err(e) => {
                eprintln!("\nerror: {e}");
                std::process::exit(1);
            }
            _ => {}
        }
    }
}
