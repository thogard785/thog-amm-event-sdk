use event_driven_sdk::{Address, Config, EventDrivenSdk, PoolModel, U256};
fn quote(model: &PoolModel, input: Address, output: Address, amount: U256) {
    match model.quote_exact_input(input, output, amount) {
        Ok(q) => println!(
            "block={} hash={:?} amount_out={} last_posted_block={}",
            model.state().block.number,
            model.state().block.hash,
            q.amount_out,
            q.last_posted_block
        ),
        Err(error) => eprintln!(
            "block={} quote unavailable: {error}",
            model.state().block.number
        ),
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::var("TOKEN_IN")?.parse()?;
    let output = std::env::var("TOKEN_OUT")?.parse()?;
    let amount = std::env::var("AMOUNT_IN")?.parse()?;
    let mut sdk = EventDrivenSdk::connect(
        std::env::var("THOGAMM_HTTP_RPC")?,
        std::env::var("THOGAMM_WS_RPC")?,
        std::env::var("THOGAMM_PROXY")?.parse()?,
        Config::default(),
    )
    .await?;
    quote(sdk.model(), input, output, amount);
    loop {
        let update = sdk.next_update().await?;
        eprintln!("synchronization: {update:?}");
        quote(sdk.model(), input, output, amount);
    }
}
