//! Word list and derivation for Proof of Inference.
//!
//! 200 common English words (4-8 letters), used to derive
//! required words from the challenge seed deterministically.

pub const WORDLIST_SIZE: usize = 200;
pub const MAX_REQUIRED: usize = 8;
pub const MAX_WORD_LEN: usize = 8;

pub const WORDLIST: [&str; WORDLIST_SIZE] = [
    // Nouns (70)
    "time","life","world","place","water","light","house","music","power","dream",
    "heart","earth","ocean","river","cloud","stone","flame","voice","night","field",
    "space","brain","truth","peace","storm","tower","plant","metal","glass","wheel",
    "bridge","forest","garden","market","island","desert","silver","shadow","spirit","nature",
    "energy","future","memory","moment","season","winter","summer","signal","system","design",
    "method","reason","answer","letter","person","animal","flower","morning","evening","journey",
    "history","culture","balance","freedom","pattern","shelter","surface","chapter","element","silence",
    // Verbs (50)
    "think","learn","build","write","speak","dance","climb","watch","shine","carry",
    "drive","paint","teach","reach","solve","share","trust","guide","shape","craft",
    "chase","drift","weave","bloom","grasp","shift","sweep","trace","wander","gather",
    "create","follow","listen","notice","wonder","happen","become","remain","travel","return",
    "search","reveal","explore","imagine","connect","protect","reflect","develop","consider","discover",
    // Adjectives (50)
    "bright","quiet","gentle","strong","simple","hidden","golden","silent","frozen","bitter",
    "tender","vivid","subtle","fierce","humble","steady","clever","honest","broken","sacred",
    "unique","global","active","native","smooth","narrow","liquid","mental","social","visual",
    "formal","casual","proper","remote","secure","stable","cosmic","ancient","modern","natural",
    "digital","central","special","private","perfect","strange","careful","curious","distant","endless",
    // Adverbs (30)
    "often","never","always","slowly","deeply","gently","simply","nearly","barely","mostly",
    "partly","surely","truly","fully","quite","still","maybe","hence","twice","ahead",
    "apart","aside","along","after","again","early","later","since","almost","around",
];

/// Derived required words (fixed-size, no heap).
pub struct RequiredWords {
    pub words: [[u8; MAX_WORD_LEN]; MAX_REQUIRED],
    pub lens: [usize; MAX_REQUIRED],
    pub count: usize,
}

/// Map difficulty to required word count.
fn word_count_for_difficulty(difficulty: u64) -> usize {
    if difficulty <= 10 { 3 }
    else if difficulty <= 15 { 4 }
    else if difficulty <= 20 { 5 }
    else if difficulty <= 30 { 6 }
    else if difficulty <= 40 { 7 }
    else { 8 }
}

/// Derive required words deterministically from challenge seed and difficulty.
pub fn derive_words(seed: &[u8; 32], difficulty: u64) -> RequiredWords {
    let count = word_count_for_difficulty(difficulty);

    let mut result = RequiredWords {
        words: [[0u8; MAX_WORD_LEN]; MAX_REQUIRED],
        lens: [0; MAX_REQUIRED],
        count,
    };

    let mut used = [false; WORDLIST_SIZE];

    let mut i = 0;
    while i < count {
        let raw = ((seed[i * 2] as u16) << 8) | (seed[i * 2 + 1] as u16);
        let mut idx = (raw as usize) % WORDLIST_SIZE;

        // Skip duplicates
        let mut tries = 0;
        while used[idx] && tries < WORDLIST_SIZE {
            idx = (idx + 1) % WORDLIST_SIZE;
            tries += 1;
        }
        if tries >= WORDLIST_SIZE {
            result.count = i;
            break;
        }

        used[idx] = true;
        let word = WORDLIST[idx].as_bytes();
        let len = word.len().min(MAX_WORD_LEN);
        let mut j = 0;
        while j < len {
            result.words[i][j] = word[j];
            j += 1;
        }
        result.lens[i] = len;

        i += 1;
    }

    result
}
