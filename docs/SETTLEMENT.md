# Settlement integration


The generic Rust ABI includes `makerSwapExactInputCall`. To construct direct
exact-input calldata:

```rust
use alloy_sol_types::SolCall;
use event_driven_sdk::model::abi::makerSwapExactInputCall;

let calldata = makerSwapExactInputCall {
    tokenIn: sell_token,
    tokenOut: buy_token,
    amountIn: sell_amount,
    minAmountOut: minimum_accepted_output,
    recipient,
    deadlineBlock: inclusive_block_deadline,
}.abi_encode();
```

Send to the configured ThogAMM **proxy**, with native value zero. The direct
caller must hold `tokenIn` and approve the proxy: settlement pulls `amountIn`
from `msg.sender` using `transferFrom`, then transfers output to `recipient`.
This is not a prefund-and-swap or callback interface. `deadlineBlock` is a block
number, not a timestamp. Amount and deadline constraints belong to the final
transaction. Encoding the call performs no RPC and sends no transaction.

Use wrapped native tokens; native-coin wrapping/unwrapping belongs to the
aggregator. Fee-on-transfer/rebasing behavior is not modeled as an ordinary
ERC-20 fill. The SDK's execution quote does not include an aggregator's own
fee or external fee-collector behavior. `swapWithExtraFees` has a distinct
fee-collector contract interface and timestamp expiry; it is not interchangeable
with the direct settlement binding above.

An exact-output quote supplies the minimum required input for the contract's
Buy solver. The direct exact-input entry point can use that input and the desired
output as its minimum, but may deliver integer surplus. The ERC-7815 Buy entry
point transfers the exact requested output but has no native maximum-input or
deadline arguments; an aggregator using it must enforce its protections in its
adapter. No SDK result substitutes for final transaction simulation and slippage
enforcement.
