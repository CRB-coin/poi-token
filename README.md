# CRB Token — Proof of Inference Mining

A Solana-based mining token where miners must generate natural language text containing specific words and solve a Proof-of-Work challenge.

## Mainnet Deployment

| Item | Value |
|------|-------|
| Program ID | `Aio7qosxjY32JuFfSrbpdv2kqYu3MF6YynPdai22HMAg` |
| Token Mint | `G3NiS1ijZtTagpMEwbGVsVGQbCMRmPRpqFjgARnzSEQS` |
| MineState PDA | `7cPxr8BoimP3WsxEPrcLxiqJJiCtPwHJkDJnXenMFagP` |
| Decimals | 3 |
| Max Supply | 2,100,000,000,000,000,000 (2.1 quadrillion raw units) |
| Initial Reward | 5,000,000,000,000 per solution (5 billion CRB raw units) |
| Halving Interval | Every 210,000 solutions |
| Epoch Duration | 600 seconds (10 minutes) |
| Target Solutions | 50 per epoch |
| Difficulty Range | 4 - 250 |

## How It Works

### Mining Cycle

1. **Read State** — Fetch `mine_state` to get current epoch, difficulty, and challenge seed
2. **Derive Words** — Deterministically derive required words from the challenge seed
3. **Generate Text** — Create natural language text (256-800 bytes) containing all required words in order
4. **Proof of Work** — Find a nonce such that `keccak256(challenge_seed | miner_key | text | "||" | nonce)` has enough leading zero bits
5. **Submit Solution** — Submit the text + nonce on-chain (creates a Solution PDA)
6. **Advance Epoch** — After epoch ends, the crank advances to the next epoch
7. **Claim Reward** — Miners claim their CRB tokens (Solution PDA is closed, rent returned)

### Text Verification

The on-chain program performs a single O(n) pass with zero heap allocation:

- Length: 256-800 bytes
- Required words must appear in order as whole words with ≥40 byte gaps
- Vowel ratio 15%-55%, space ratio 10%-30%
- Max 5 consecutive consonants, average consonant cluster ≤3.5
- Common bigram frequency (th, he, in, er, an) ≥ len/80
- Byte diversity ≥30 distinct bytes
- Sentence structure: capital start, punctuation end
- At least 3 sentences, at least 1 question
- Mix of short (≤10 words) and long (≥20 words) sentences
- No duplicate sentences (FNV-1a hash, max 50 sentences)

### Difficulty Adjustment

Difficulty adjusts each epoch based on solution count vs target (50):
- Too many solutions → difficulty increases (log2 dampened, max +5)
- Too few solutions → difficulty decreases (log2 dampened, max -5)
- Zero solutions → max decrease (-5)
- Range: 4 (minimum) to 250 (maximum)

### Reward Schedule (Halving)

| Total Mined | Reward per Solution |
|-------------|-------------------|
| 0 - 209,999 | 5,000,000,000,000 |
| 210,000 - 419,999 | 2,500,000,000,000 |
| 420,000 - 629,999 | 1,250,000,000,000 |
| ... | Halves every 210,000 solutions |

## Architecture

Zero write-lock contention design:

- `submit_solution` reads `mine_state` as **read-only** — no shared write locks
- Each solution creates its own PDA: `seeds = ["solution", miner_key, epoch_bytes]`
- Unlimited parallel miners with zero transaction conflicts
- Solution counting is passed by the crank during `advance_epoch`

### Instructions

| Instruction | Description |
|-------------|-------------|
| `initialize` | Create MineState PDA and token Mint |
| `submit_solution(text, nonce)` | Submit a mining solution |
| `advance_epoch(solution_count)` | Advance to next epoch, adjust difficulty |
| `claim_reward` | Claim CRB reward, close Solution PDA, recover rent |
| `close_expired` | Close expired unclaimed solutions (500+ epochs old) |

## Quick Start

### Prerequisites

- [Solana CLI](https://docs.solanalabs.com/cli/install)
- [Node.js](https://nodejs.org/) v18+
- A Solana wallet with SOL for transaction fees

### 1. Create a Miner Wallet

```bash
solana-keygen new -o miner-keypair.json
solana address -k miner-keypair.json
```

Fund this address with a small amount of SOL for transaction fees (~0.01 SOL).

### 2. Run the Miner

```bash
cd miner
npm install
npx ts-node --transpile-only mainnet-miner.ts
```

By default, the miner reads `./miner-keypair.json`. To use a different wallet:

```bash
export KEYPAIR=/path/to/your/keypair.json
npx ts-node --transpile-only mainnet-miner.ts
```

To use a custom RPC endpoint:

```bash
export RPC_URL=https://your-rpc-endpoint.com
```

### 3. Switch Receiving Wallet

The mining wallet is the receiving wallet — CRB tokens are sent to the wallet that submitted the solution.

To switch your receiving wallet:

1. Stop the miner
2. Set the new keypair:
   ```bash
   export KEYPAIR=/path/to/new-wallet.json
   ```
3. Restart the miner

Previously submitted but unclaimed solutions will still be claimed to the old wallet. New solutions will go to the new wallet.

## Word List

200 common English words (4-8 letters) are used for text requirements. The number of required words scales with difficulty:

| Difficulty | Required Words |
|-----------|---------------|
| ≤ 10 | 3 |
| ≤ 15 | 4 |
| ≤ 20 | 5 |
| ≤ 30 | 6 |
| ≤ 40 | 7 |
| > 40 | 8 |

## License

MIT
