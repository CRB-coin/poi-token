//! Text constraint verification for Proof of Inference.
//!
//! Single O(n) pass, no_std compatible, zero heap allocation.
//! Checks: length, required words (with word boundaries), sentence structure,
//! vowel/space ratios, consonant clusters, bigram frequency, byte diversity.

/// FNV-1a 64-bit hash for sentence dedup (two seeds → 128-bit effective)
fn simple_hash(data: &[u8]) -> (u64, u64) {
    let mut h1: u64 = 0xcbf29ce484222325;
    let mut h2: u64 = 0x6c62272e07bb0142;
    let mut i = 0;
    while i < data.len() {
        let b = data[i] as u64;
        h1 ^= b;
        h1 = h1.wrapping_mul(0x100000001b3);
        h2 ^= b;
        h2 = h2.wrapping_mul(0x100000001b3);
        i += 1;
    }
    (h1, h2)
}

#[inline(always)]
fn is_alpha(b: u8) -> bool {
    (b >= b'A' && b <= b'Z') || (b >= b'a' && b <= b'z')
}

#[inline(always)]
fn to_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' { b + 32 } else { b }
}

#[inline(always)]
fn is_vowel_lower(b: u8) -> bool {
    matches!(b, b'a' | b'e' | b'i' | b'o' | b'u')
}

#[inline(always)]
fn is_whitespace(b: u8) -> bool {
    b == b' ' || b == b'\n' || b == b'\t' || b == b'\r'
}

#[inline(always)]
fn is_sentence_end(b: u8) -> bool {
    matches!(b, b'.' | b'!' | b'?')
}

/// Verify text meets all natural-language constraints.
///
/// `required_words`: must appear in order, as whole words, with ≥40 byte gap.
pub fn verify_text(text: &[u8], required_words: &[&[u8]]) -> bool {
    let len = text.len();

    // ── 1. Length: 256–800 bytes ──
    // (Solana tx limit is 1232 bytes; ~900 usable for text after overhead)
    if len < 256 || len > 800 {
        return false;
    }

    // ── State variables ──
    let mut letter_count: u32 = 0;
    let mut vowel_count: u32 = 0;
    let mut space_count: u32 = 0;

    // Byte diversity: 256-bit bitmap in 4 × u64
    let mut bmap: [u64; 4] = [0; 4];

    // Bigrams (case-insensitive)
    let mut prev_lower: u8 = 0;
    let mut bg_th: u32 = 0;
    let mut bg_he: u32 = 0;
    let mut bg_in: u32 = 0;
    let mut bg_er: u32 = 0;
    let mut bg_an: u32 = 0;

    // Consonant clusters
    let mut cons_run: u32 = 0;
    let mut cons_max: u32 = 0;
    let mut cons_total: u32 = 0;
    let mut cons_count: u32 = 0;

    // Sentence tracking
    let mut words_in_sent: u32 = 0;
    let mut in_word: bool = false;
    let mut sent_count: u32 = 0;
    let mut has_question: bool = false;
    let mut has_short: bool = false;   // ≤10 words
    let mut has_long: bool = false;    // ≥20 words
    let mut sent_start: usize = 0;
    let mut sent_started: bool = false;

    // Sentence dedup: store up to 50 hashes
    let mut sent_hashes: [(u64, u64); 50] = [(0, 0); 50];
    let mut hash_count: usize = 0;

    // Required word matching
    let rw_total = required_words.len();
    let mut rw_idx: usize = 0;       // which required word we're looking for
    let mut rw_match: usize = 0;     // bytes matched so far in current word
    let mut rw_match_start: usize = 0; // where current match started
    let mut last_rw_end: usize = 0;  // end position of last matched word
    let mut has_rw_match: bool = false;

    // ── Main loop ──
    let mut i: usize = 0;
    while i < len {
        let b = text[i];
        let lower = to_lower(b);
        let alpha = is_alpha(b);
        let vowel = alpha && is_vowel_lower(lower);
        let space = b == b' ';
        let ws = is_whitespace(b);
        let sent_end = is_sentence_end(b);

        // ASCII only — reject bytes > 127
        if b > 127 {
            return false;
        }

        // Byte diversity
        bmap[(b >> 6) as usize] |= 1u64 << (b & 63);

        // Letter / vowel / space counts
        if alpha {
            letter_count += 1;
            if vowel { vowel_count += 1; }
        }
        if space { space_count += 1; }

        // Consonant cluster tracking
        if alpha && !vowel {
            cons_run += 1;
        } else if cons_run > 0 {
            if cons_run > cons_max { cons_max = cons_run; }
            cons_total += cons_run;
            cons_count += 1;
            cons_run = 0;
        }

        // Bigram detection
        if i > 0 {
            match (prev_lower, lower) {
                (b't', b'h') => bg_th += 1,
                (b'h', b'e') => bg_he += 1,
                (b'i', b'n') => bg_in += 1,
                (b'e', b'r') => bg_er += 1,
                (b'a', b'n') => bg_an += 1,
                _ => {}
            }
        }
        prev_lower = lower;

        // Word tracking within sentence
        if ws || sent_end {
            in_word = false;
        } else if !in_word {
            in_word = true;
            words_in_sent += 1;
        }

        // Sentence start position (skip leading whitespace)
        if !sent_started && !ws && !sent_end {
            sent_start = i;
            sent_started = true;
        }

        // ── Required word matching (with word boundary check) ──
        if rw_idx < rw_total {
            let rw = required_words[rw_idx];
            if rw.len() > 0 && lower == to_lower(rw[rw_match]) {
                if rw_match == 0 {
                    rw_match_start = i;
                }
                rw_match += 1;
                if rw_match == rw.len() {
                    // Full match — check word boundaries
                    let before_ok = rw_match_start == 0 || !is_alpha(text[rw_match_start - 1]);
                    let after_ok = i + 1 >= len || !is_alpha(text[i + 1]);

                    if before_ok && after_ok {
                        // Check minimum gap from previous match
                        if has_rw_match && rw_match_start < last_rw_end + 40 {
                            // Gap too small — skip this occurrence, fall through to reset
                        } else {
                            last_rw_end = i + 1;
                            has_rw_match = true;
                            rw_idx += 1;
                        }
                    }
                    // Reset and check if current byte starts new match
                    // (handles both: boundary fail → retry same word,
                    //  and success → check next word)
                    rw_match = 0;
                    if rw_idx < rw_total {
                        let rw_next = required_words[rw_idx];
                        if rw_next.len() > 0 && lower == to_lower(rw_next[0]) {
                            rw_match_start = i;
                            rw_match = 1;
                        }
                    }
                }
            } else if rw_match > 0 {
                // Match interrupted — reset and check if current byte starts new match
                rw_match = 0;
                if rw.len() > 0 && lower == to_lower(rw[0]) {
                    rw_match_start = i;
                    rw_match = 1;
                }
            }
        }

        // ── Sentence end ──
        if sent_end && words_in_sent > 0 && sent_started {
            // Word count bounds: 5–35
            if words_in_sent < 5 || words_in_sent > 35 {
                return false;
            }
            if b == b'?' { has_question = true; }
            if words_in_sent <= 10 { has_short = true; }
            if words_in_sent >= 20 { has_long = true; }

            // Sentence dedup
            if hash_count < 50 {
                let h = simple_hash(&text[sent_start..=i]);
                let mut j = 0;
                while j < hash_count {
                    if sent_hashes[j] == h {
                        return false; // duplicate sentence
                    }
                    j += 1;
                }
                sent_hashes[hash_count] = h;
                hash_count += 1;
            }
            sent_count += 1;

            // Reset sentence state
            words_in_sent = 0;
            in_word = false;
            sent_started = false;
        }

        i += 1;
    }

    // Flush trailing consonant cluster
    if cons_run > 0 {
        if cons_run > cons_max { cons_max = cons_run; }
        cons_total += cons_run;
        cons_count += 1;
    }

    // ── Post-loop checks ──

    // All required words found
    if rw_idx < rw_total { return false; }

    // Sentence structure
    if sent_count < 2 { return false; }
    if !has_question { return false; }
    if !has_short { return false; }
    if !has_long { return false; }

    // Vowel ratio: 30–48% of letters
    if letter_count == 0 { return false; }
    let vc = vowel_count as u64;
    let lc = letter_count as u64;
    if vc * 100 < 30 * lc || vc * 100 > 48 * lc { return false; }

    // Space ratio: 12–22% of total bytes
    let sc = space_count as u64;
    let total = len as u64;
    if sc * 100 < 12 * total || sc * 100 > 22 * total { return false; }

    // Consonant clusters: max ≤5, avg <2.5
    if cons_max > 5 { return false; }
    if cons_count > 0 && cons_total * 10 >= 25 * cons_count { return false; }

    // Bigrams: th/he/in/er/an each ≥2
    if bg_th < 2 || bg_he < 2 || bg_in < 2 || bg_er < 2 || bg_an < 2 { return false; }

    // Byte diversity: ≥28 unique values
    // (natural English text has ~31-34: 22-25 lowercase + 3-5 uppercase + 4-6 punctuation)
    let unique = bmap[0].count_ones() + bmap[1].count_ones()
               + bmap[2].count_ones() + bmap[3].count_ones();
    if unique < 28 { return false; }

    true
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    fn natural_text() -> Vec<u8> {
        let text = "The weather in the morning was rather interesting and \
            pleasant for an early spring day in the northern hemisphere. \
            Have you ever wondered whether the inner workings of nature \
            can truly be understood through simple observation and careful \
            thinking about the patterns that emerge in everything around us? \
            The ancient trees in the garden were standing tall and their \
            branches reached toward the bright sky above. \
            The morning air felt crisp and fresh. \
            Another interesting thing happened when the river began to \
            change direction and the water flowed in an entirely different \
            manner than before. \
            Is there anything more beautiful than a quiet evening spent \
            reading by the fireplace?";
        text.as_bytes().to_vec()
    }

    #[test]
    fn test_natural_passes() {
        let text = natural_text();
        let words: &[&[u8]] = &[b"weather", b"nature", b"ancient"];
        assert!(verify_text(&text, words), "Natural text should pass, len={}", text.len());
    }

    #[test]
    fn test_too_short() {
        assert!(!verify_text(b"Hello world.", &[]));
    }

    #[test]
    fn test_duplicate_sentences() {
        let s1 = "The weather in the morning was rather interesting and pleasant. ";
        let q = "Have you ever wondered whether the inner workings of nature can truly be understood? ";
        let long = "The ancient trees in the garden were standing tall and their branches reached toward the bright sky creating an interesting pattern. ";
        let mut t = String::new();
        t.push_str(s1); t.push_str(q); t.push_str(long); t.push_str(s1); // dup!
        while t.len() < 256 { t.push_str("Another filler sentence in the text here today. "); }
        assert!(!verify_text(t.as_bytes(), &[]), "Duplicate sentences should fail");
    }

    #[test]
    fn test_no_question() {
        let t = "The weather in the morning was rather interesting and pleasant for an early spring day. \
            The ancient trees in the garden were standing tall and their branches reached toward the bright sky above and beyond the hills. \
            Another interesting thing happened when the river began to change direction and the water flowed in an entirely different manner than before. \
            The evening settled over the land.";
        let padded = format!("{} {}", t, "More filler text about the interesting weather and the ancient garden path. ".repeat(2));
        assert!(!verify_text(padded.as_bytes(), &[]), "Missing question should fail");
    }

    #[test]
    fn test_missing_required_word() {
        let text = natural_text();
        let words: &[&[u8]] = &[b"weather", b"blockchain", b"ancient"];
        assert!(!verify_text(&text, words), "Missing required word should fail");
    }

    #[test]
    fn test_wrong_word_order() {
        let text = natural_text();
        let words: &[&[u8]] = &[b"ancient", b"nature"];
        assert!(!verify_text(&text, words), "Wrong word order should fail");
    }

    #[test]
    fn test_word_boundary() {
        // "other" contains "the" but should NOT match required word "the"
        // when checking word boundaries
        let t = "Another other thing happened in the morning when the weather changed and everything \
            looked rather different from what the ancient stories had described in their pages. \
            Is there anything more interesting than discovering the hidden patterns in nature? \
            The answer to this question remains unclear even today. \
            Sometimes the world reveals its secrets only to those who listen carefully and patiently.";
        let padded = format!("{} {}", t, "The garden path led through the forest and over the bridge. ");
        // "other" appears before standalone "the" — should still find standalone "the"
        let words: &[&[u8]] = &[b"the"];
        // This should pass because standalone "the" exists
        if padded.len() >= 256 {
            assert!(verify_text(padded.as_bytes(), words), "Word boundary: standalone 'the' should match");
        }
    }

    #[test]
    fn test_gibberish() {
        let mut g = Vec::with_capacity(300);
        let cons = b"bcdfghjklmnpqrstvwxyz";
        for i in 0u16..300 {
            if i % 7 == 0 { g.push(b' '); }
            else if i % 50 == 49 { g.push(b'.'); }
            else { g.push(cons[(i as usize) % cons.len()]); }
        }
        assert!(!verify_text(&g, &[]), "Gibberish should fail");
    }
}
