//! Follow the pool from a Monad WebSocket subscription and print one quote
//! per finalized block.
//!
//!     MONAD_RPC=https://rpc.monad.xyz MONAD_WS=wss://rpc.monad.xyz \
//!     TOKEN_IN=0x... TOKEN_OUT=0x... AMOUNT_IN=100000000 cargo run --example subscribe
use thogamm_events::{Subscriber, U256};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool_address = "0x80c74517BCC2D67fFE02D3ED886796272F647210".parse()?;
    let token_in = std::env::var("TOKEN_IN")?.parse()?;
    let token_out = std::env::var("TOKEN_OUT")?.parse()?;
    let amount_in: U256 = std::env::var("AMOUNT_IN")?.parse()?;
    let (http, ws) = (std::env::var("MONAD_RPC")?, std::env::var("MONAD_WS")?);

    loop {
        let mut subscriber = Subscriber::connect(&http, &ws, pool_address).await?;
        loop {
            let pool = subscriber.pool();
            let block = pool.snapshot().block.number;
            // A pair that cannot trade right now is an ordinary answer, not a failure.
            match pool
                .pair(token_in, token_out)
                .and_then(|pair| pair.maker_quote_exact_input(amount_in))
            {
                Ok(quote) => println!(
                    "block {block}: {} out, prices from block {}",
                    quote.amount_out, quote.price_block
                ),
                Err(error) => println!("block {block}: {error}"),
            }
            // A missed block cannot be recovered from logs: read the pool again.
            if let Err(error) = subscriber.next_update().await {
                eprintln!("subscription ended: {error}; reconnecting");
                break;
            }
        }
    }
}
