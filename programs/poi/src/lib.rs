use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak;
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount};

pub mod verify;
pub mod words;

declare_id!("Aio7qosxjY32JuFfSrbpdv2kqYu3MF6YynPdai22HMAg");

// ============================================================
// Constants
// ============================================================

const MAX_SUPPLY: u64 = 2_100_000_000_000_000 * 1_000; // 2.1 quadrillion × 10^3 (3 decimals)
const INITIAL_REWARD: u64 = 5_000_000_000 * 1_000;      // 5 billion CRB × 10^3
const HALVING_INTERVAL: u64 = 210_000;
const EPOCH_DURATION: i64 = 600;                         // 10 min production epoch
const TARGET_SOLUTIONS: u64 = 50;
const INITIAL_DIFFICULTY: u64 = 8;                       // 8 for testing (20 production)
const MAX_DIFFICULTY: u64 = 250;
const MIN_DIFFICULTY: u64 = 4;                           // lowered from 8 for early-stage UX
const MAX_DIFFICULTY_ADJ: u64 = 5;                       // max ±5 per epoch (bounds crank trust)
const CLAIM_EXPIRY_EPOCHS: u64 = 500;                    // unclaimed solutions expire after 500 epochs

// ============================================================
// Program
// ============================================================

#[program]
pub mod proof_of_inference {
    use super::*;

    /// Initialize the mining state and create the SPL token mint.
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let clock = Clock::get()?;
        let mine_state_key = ctx.accounts.mine_state.key();
        let mint_key = ctx.accounts.mint.key();
        let bump = ctx.bumps.mine_state;

        let seed_input = [
            clock.slot.to_le_bytes().as_ref(),
            clock.unix_timestamp.to_le_bytes().as_ref(),
            mine_state_key.as_ref(),
        ]
        .concat();
        let challenge_seed = keccak::hash(&seed_input).to_bytes();

        let state = &mut ctx.accounts.mine_state;
        state.total_mined = 0;
        state.difficulty = INITIAL_DIFFICULTY;
        state.challenge_seed = challenge_seed;
        state.epoch_number = 0;
        state.epoch_start_time = clock.unix_timestamp;
        state.epoch_end_time = clock.unix_timestamp + EPOCH_DURATION;
        state.solutions_in_epoch = 0;
        state.settled_in_epoch = 0;
        state.total_supply = 0;
        state.mint = mint_key;
        state.crank_authority = ctx.accounts.payer.key();
        state.bump = bump;

        Ok(())
    }

    /// Submit a mining solution.
    ///
    /// Phase 1 key change: mine_state is READ-ONLY.
    /// No shared state writes — each submit only creates a unique Solution PDA.
    /// This makes all submits fully parallelizable on Solana's SVM.
    pub fn submit_solution(ctx: Context<SubmitSolution>, text: String, nonce: u64) -> Result<()> {
        let clock = Clock::get()?;

        // ── Read state (mine_state is read-only, no write lock) ──
        let challenge_seed = ctx.accounts.mine_state.challenge_seed;
        let difficulty = ctx.accounts.mine_state.difficulty;
        let epoch_number = ctx.accounts.mine_state.epoch_number;
        let epoch_end_time = ctx.accounts.mine_state.epoch_end_time;
        let total_supply = ctx.accounts.mine_state.total_supply;

        // ── Epoch must be active ──
        require!(
            clock.unix_timestamp < epoch_end_time,
            ErrorCode::EpochEnded
        );

        // ── Supply cap ──
        require!(total_supply < MAX_SUPPLY, ErrorCode::MaxSupplyReached);

        // ── NO MAX_SOLUTIONS check — difficulty naturally regulates throughput ──

        // ── Derive required words ──
        let rw = words::derive_words(&challenge_seed, difficulty);
        let w0 = &rw.words[0][..rw.lens[0]];
        let w1 = &rw.words[1][..rw.lens[1]];
        let w2 = &rw.words[2][..rw.lens[2]];
        let w3 = &rw.words[3][..rw.lens[3]];
        let w4 = &rw.words[4][..rw.lens[4]];
        let w5 = &rw.words[5][..rw.lens[5]];
        let w6 = &rw.words[6][..rw.lens[6]];
        let w7 = &rw.words[7][..rw.lens[7]];
        let all_words: [&[u8]; 8] = [w0, w1, w2, w3, w4, w5, w6, w7];
        let active_words = &all_words[..rw.count];

        // ── Verify text constraints ──
        require!(
            verify::verify_text(text.as_bytes(), active_words),
            ErrorCode::InvalidText
        );

        // ── Compute hash ──
        let miner_key = ctx.accounts.miner.key();
        let nonce_bytes = nonce.to_le_bytes();
        let hash = keccak::hashv(&[
            &challenge_seed,
            miner_key.as_ref(),
            text.as_bytes(),
            b"||",
            &nonce_bytes,
        ]);
        let hash_bytes = hash.to_bytes();

        // ── Verify PoW difficulty ──
        require!(
            check_difficulty(&hash_bytes, difficulty),
            ErrorCode::InsufficientDifficulty
        );

        // ── Write Solution PDA (only per-miner state, no shared writes) ──
        let solution = &mut ctx.accounts.solution;
        solution.miner = miner_key;
        solution.epoch = epoch_number;
        solution.nonce = nonce;
        solution.hash = hash_bytes;
        solution.bump = ctx.bumps.solution;

        // ── NO state.solutions_in_epoch += 1 ──
        // This is the key Phase 1 change: submit writes ZERO shared state.
        // Solution count is indexed off-chain by the Crank service.

        Ok(())
    }

    /// Claim reward for a submitted solution.
    ///
    /// Replaces the old `settle` instruction. Key differences:
    /// - Miners call this themselves (self-service, no time pressure)
    /// - Solutions expire after CLAIM_EXPIRY_EPOCHS (unclaimed rent is forfeited)
    /// - Anyone CAN call this on behalf of a miner (permissionless), but
    ///   reward + rent always go to the solution's miner.
    pub fn claim(ctx: Context<Claim>) -> Result<()> {
        let clock = Clock::get()?;

        // ── Read state ──
        let current_epoch = ctx.accounts.mine_state.epoch_number;
        let epoch_end_time = ctx.accounts.mine_state.epoch_end_time;
        let total_mined = ctx.accounts.mine_state.total_mined;
        let total_supply = ctx.accounts.mine_state.total_supply;
        let bump = ctx.accounts.mine_state.bump;
        let solution_epoch = ctx.accounts.solution.epoch;

        // ── Solution's epoch must have ended ──
        let epoch_over = if solution_epoch < current_epoch {
            true
        } else if solution_epoch == current_epoch {
            clock.unix_timestamp >= epoch_end_time
        } else {
            false
        };
        require!(epoch_over, ErrorCode::EpochNotEnded);

        // ── Not expired ──
        require!(
            current_epoch < solution_epoch.saturating_add(CLAIM_EXPIRY_EPOCHS),
            ErrorCode::ClaimExpired
        );

        // ── Calculate reward ──
        let reward = calculate_reward(total_mined);
        let actual_reward = reward.min(MAX_SUPPLY.saturating_sub(total_supply));

        // ── CPI: mint tokens to miner ──
        if actual_reward > 0 {
            let seeds = &[b"mine_state".as_ref(), &[bump]];
            let signer_seeds = &[&seeds[..]];

            token::mint_to(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    MintTo {
                        mint: ctx.accounts.mint.to_account_info(),
                        to: ctx.accounts.recipient_token_account.to_account_info(),
                        authority: ctx.accounts.mine_state.to_account_info(),
                    },
                    signer_seeds,
                ),
                actual_reward,
            )?;
        }

        // ── Update state ──
        let state = &mut ctx.accounts.mine_state;
        state.total_mined += 1;
        if actual_reward > 0 {
            state.total_supply += actual_reward;
        }

        // Solution PDA closed by Anchor `close` constraint → rent to rent_recipient (miner)

        Ok(())
    }

    /// Advance to the next epoch.
    ///
    /// Called by Crank service after epoch ends.
    /// Crank indexes Solution PDAs off-chain and passes solution_count.
    /// Difficulty adjustment is capped at ±5 per epoch, bounding crank trust risk.
    pub fn advance_epoch(ctx: Context<AdvanceEpoch>, solution_count: u64) -> Result<()> {
        let clock = Clock::get()?;
        let state = &mut ctx.accounts.mine_state;

        // ── Current epoch must have ended ──
        require!(
            clock.unix_timestamp >= state.epoch_end_time,
            ErrorCode::EpochNotEnded
        );

        // ── Proportional difficulty adjustment (log2 dampened, ±5 capped) ──
        let target = TARGET_SOLUTIONS;

        if solution_count > target + target / 5 {
            // Too many solutions → increase difficulty
            let ratio = solution_count / target;
            let increase = log2_ceil(ratio).max(1).min(MAX_DIFFICULTY_ADJ);
            state.difficulty = state.difficulty.saturating_add(increase).min(MAX_DIFFICULTY);
        } else if solution_count == 0 {
            // Empty epoch → decrease by max amount
            state.difficulty = state.difficulty.saturating_sub(MAX_DIFFICULTY_ADJ).max(MIN_DIFFICULTY);
        } else if solution_count < target.saturating_sub(target / 5) {
            // Too few solutions → decrease difficulty
            let ratio = target / solution_count;
            let decrease = log2_ceil(ratio).max(1).min(MAX_DIFFICULTY_ADJ);
            state.difficulty = state.difficulty.saturating_sub(decrease).max(MIN_DIFFICULTY);
        }
        // If within ±20% of target, difficulty stays the same

        // ── Store solution count for record-keeping ──
        state.solutions_in_epoch = solution_count;

        // ── New challenge seed ──
        let seed_input = [
            state.challenge_seed.as_ref(),
            clock.unix_timestamp.to_le_bytes().as_ref(),
            state.epoch_number.to_le_bytes().as_ref(),
            clock.slot.to_le_bytes().as_ref(),
        ]
        .concat();
        state.challenge_seed = keccak::hash(&seed_input).to_bytes();

        // ── Advance epoch ──
        state.epoch_number += 1;
        state.epoch_start_time = clock.unix_timestamp;
        state.epoch_end_time = clock.unix_timestamp + EPOCH_DURATION;
        state.settled_in_epoch = 0;

        Ok(())
    }

    /// Close an expired Solution PDA and reclaim rent.
    ///
    /// Anyone can call this after CLAIM_EXPIRY_EPOCHS have passed.
    /// Rent goes to the caller as incentive for cleanup.
    /// The miner's reward is forfeited.
    pub fn close_expired(ctx: Context<CloseExpired>) -> Result<()> {
        let current_epoch = ctx.accounts.mine_state.epoch_number;
        let solution_epoch = ctx.accounts.solution.epoch;

        require!(
            current_epoch >= solution_epoch.saturating_add(CLAIM_EXPIRY_EPOCHS),
            ErrorCode::NotExpired
        );

        // Solution PDA closed by Anchor `close` constraint → rent to closer

        Ok(())
    }

    /// Transfer crank authority to a new address.
    pub fn set_crank_authority(ctx: Context<SetCrankAuthority>, new_authority: Pubkey) -> Result<()> {
        ctx.accounts.mine_state.crank_authority = new_authority;
        Ok(())
    }

    /// Create token metadata via Metaplex Token Metadata program.
    /// Only callable by crank authority.
    pub fn create_metadata(
        ctx: Context<CreateMetadata>,
        name: String,
        symbol: String,
        uri: String,
    ) -> Result<()> {
        let bump = ctx.accounts.mine_state.bump;
        let seeds = &[b"mine_state".as_ref(), &[bump]];
        let signer_seeds = &[&seeds[..]];

        let metadata_accounts = mpl_token_metadata::instructions::CreateMetadataAccountV3CpiAccounts {
            metadata: &ctx.accounts.metadata.to_account_info(),
            mint: &ctx.accounts.mint.to_account_info(),
            mint_authority: &ctx.accounts.mine_state.to_account_info(),
            payer: &ctx.accounts.payer.to_account_info(),
            update_authority: (&ctx.accounts.mine_state.to_account_info(), true),
            system_program: &ctx.accounts.system_program.to_account_info(),
            rent: Some(&ctx.accounts.rent.to_account_info()),
        };

        let data_v2 = mpl_token_metadata::types::DataV2 {
            name,
            symbol,
            uri,
            seller_fee_basis_points: 0,
            creators: None,
            collection: None,
            uses: None,
        };

        mpl_token_metadata::instructions::CreateMetadataAccountV3Cpi::new(
            &ctx.accounts.token_metadata_program.to_account_info(),
            metadata_accounts,
            mpl_token_metadata::instructions::CreateMetadataAccountV3InstructionArgs {
                data: data_v2,
                is_mutable: true,
                collection_details: None,
            },
        ).invoke_signed(signer_seeds)?;

        Ok(())
    }
}

// ============================================================
// Helper Functions
// ============================================================

/// Check if hash meets difficulty: first `difficulty` bits must be zero.
fn check_difficulty(hash: &[u8; 32], difficulty: u64) -> bool {
    if difficulty == 0 {
        return true;
    }
    if difficulty >= 256 {
        return false;
    }
    let full_bytes = (difficulty / 8) as usize;
    let remaining_bits = (difficulty % 8) as u8;

    let mut i = 0;
    while i < full_bytes {
        if hash[i] != 0 {
            return false;
        }
        i += 1;
    }
    if remaining_bits > 0 && full_bytes < 32 {
        let mask: u8 = 0xFF << (8 - remaining_bits);
        if hash[full_bytes] & mask != 0 {
            return false;
        }
    }
    true
}

/// Reward with halving: INITIAL_REWARD >> (total_mined / HALVING_INTERVAL)
fn calculate_reward(total_mined: u64) -> u64 {
    let halvings = total_mined / HALVING_INTERVAL;
    if halvings >= 64 {
        return 0;
    }
    INITIAL_REWARD >> halvings
}

/// Integer ceiling of log2. Returns 0 for x <= 1.
/// Used for proportional difficulty adjustment dampening.
fn log2_ceil(x: u64) -> u64 {
    if x <= 1 {
        return 0;
    }
    64 - (x - 1).leading_zeros() as u64
}

// ============================================================
// Accounts
// ============================================================

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = payer,
        space = 8 + MineState::INIT_SPACE,
        seeds = [b"mine_state"],
        bump,
    )]
    pub mine_state: Account<'info, MineState>,

    #[account(
        init,
        payer = payer,
        mint::decimals = 3,
        mint::authority = mine_state,
        seeds = [b"mint"],
        bump,
    )]
    pub mint: Account<'info, Mint>,

    #[account(mut)]
    pub payer: Signer<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct SubmitSolution<'info> {
    // ── READ-ONLY: no write lock acquired ──
    // This is the key Phase 1 optimization.
    // All submits can execute in parallel since they don't write shared state.
    #[account(
        seeds = [b"mine_state"],
        bump = mine_state.bump,
    )]
    pub mine_state: Account<'info, MineState>,

    #[account(
        init,
        payer = miner,
        space = 8 + Solution::INIT_SPACE,
        seeds = [b"solution", miner.key().as_ref(), &mine_state.epoch_number.to_le_bytes()],
        bump,
    )]
    pub solution: Account<'info, Solution>,

    #[account(mut)]
    pub miner: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Claim<'info> {
    #[account(
        mut,
        seeds = [b"mine_state"],
        bump = mine_state.bump,
    )]
    pub mine_state: Account<'info, MineState>,

    #[account(
        mut,
        seeds = [b"solution", solution.miner.as_ref(), &solution.epoch.to_le_bytes()],
        bump = solution.bump,
        close = rent_recipient,
    )]
    pub solution: Account<'info, Solution>,

    #[account(
        mut,
        seeds = [b"mint"],
        bump,
    )]
    pub mint: Account<'info, Mint>,

    /// Token account to receive mined tokens (must belong to solution.miner).
    #[account(
        mut,
        token::mint = mint,
        constraint = recipient_token_account.owner == solution.miner @ ErrorCode::InvalidRecipient,
    )]
    pub recipient_token_account: Account<'info, TokenAccount>,

    /// Receives rent from closed Solution PDA (must be the miner).
    /// CHECK: validated by constraint.
    #[account(
        mut,
        constraint = rent_recipient.key() == solution.miner @ ErrorCode::InvalidRecipient,
    )]
    pub rent_recipient: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct AdvanceEpoch<'info> {
    #[account(
        mut,
        seeds = [b"mine_state"],
        bump = mine_state.bump,
    )]
    pub mine_state: Account<'info, MineState>,

    #[account(
        constraint = crank.key() == mine_state.crank_authority @ ErrorCode::Unauthorized
    )]
    pub crank: Signer<'info>,
}

#[derive(Accounts)]
pub struct SetCrankAuthority<'info> {
    #[account(
        mut,
        seeds = [b"mine_state"],
        bump = mine_state.bump,
    )]
    pub mine_state: Account<'info, MineState>,

    #[account(
        constraint = authority.key() == mine_state.crank_authority @ ErrorCode::Unauthorized
    )]
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct CloseExpired<'info> {
    #[account(
        seeds = [b"mine_state"],
        bump = mine_state.bump,
    )]
    pub mine_state: Account<'info, MineState>,

    #[account(
        mut,
        seeds = [b"solution", solution.miner.as_ref(), &solution.epoch.to_le_bytes()],
        bump = solution.bump,
        close = closer,
    )]
    pub solution: Account<'info, Solution>,

    /// Anyone can close expired solutions. Rent goes to caller as cleanup incentive.
    #[account(mut)]
    pub closer: Signer<'info>,
}
#[derive(Accounts)]
pub struct CreateMetadata<'info> {
    #[account(
        seeds = [b"mine_state"],
        bump = mine_state.bump,
    )]
    pub mine_state: Account<'info, MineState>,

    #[account(
        seeds = [b"mint"],
        bump,
    )]
    pub mint: Account<'info, Mint>,

    /// CHECK: Created by Metaplex program
    #[account(mut)]
    pub metadata: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = payer.key() == mine_state.crank_authority @ ErrorCode::Unauthorized
    )]
    pub payer: Signer<'info>,

    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,

    /// CHECK: Metaplex Token Metadata program
    #[account(address = mpl_token_metadata::ID)]
    pub token_metadata_program: UncheckedAccount<'info>,
}


// ============================================================
// State
// ============================================================

#[account]
#[derive(InitSpace)]
pub struct MineState {
    pub total_mined: u64,          // 8   — total solutions ever claimed
    pub difficulty: u64,           // 8
    pub challenge_seed: [u8; 32],  // 32
    pub epoch_number: u64,         // 8
    pub epoch_start_time: i64,     // 8
    pub epoch_end_time: i64,       // 8
    pub solutions_in_epoch: u64,   // 8   — set by crank during advance_epoch (record-keeping)
    pub settled_in_epoch: u64,     // 8   — reserved for compatibility
    pub total_supply: u64,         // 8
    pub mint: Pubkey,              // 32
    pub crank_authority: Pubkey,   // 32  — only this address can call advance_epoch
    pub bump: u8,                  // 1
}                                  // total: 161 + 8 discriminator = 169

#[account]
#[derive(InitSpace)]
pub struct Solution {
    pub miner: Pubkey,             // 32
    pub epoch: u64,                // 8
    pub nonce: u64,                // 8
    pub hash: [u8; 32],            // 32
    pub bump: u8,                  // 1
}                                  // total: 81 + 8 discriminator = 89

// ============================================================
// Errors
// ============================================================

#[error_code]
pub enum ErrorCode {
    #[msg("Text verification failed")]
    InvalidText,
    #[msg("Hash does not meet difficulty requirement")]
    InsufficientDifficulty,
    #[msg("Maximum token supply reached")]
    MaxSupplyReached,
    #[msg("Current epoch has ended, call advance_epoch first")]
    EpochEnded,
    #[msg("Epoch has not ended yet")]
    EpochNotEnded,
    #[msg("Recipient does not match solution miner")]
    InvalidRecipient,
    #[msg("Solution claim period has expired (500 epochs)")]
    ClaimExpired,
    #[msg("Solution has not expired yet")]
    NotExpired,
    #[msg("Unauthorized: not the crank authority")]
    Unauthorized,
}
