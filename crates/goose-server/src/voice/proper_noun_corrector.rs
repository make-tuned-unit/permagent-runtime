//! Post-STT proper-noun correction against Brain entity names.
//!
//! After STT produces a transcript, fuzzy-match tokens against a dictionary of
//! known proper nouns (people, projects, products) sourced from the Brain.
//! Replace close matches above a conservative threshold.
//!
//! ## Root-cause: polluted entity dictionary (tracked follow-up)
//!
//! The Brain entity dictionary contains common words and sentence fragments stored
//! as entities — a Brain entity-quality bug. This module hardens the CORRECTOR
//! against a polluted dictionary via threshold (0.95) and common-word target
//! filtering, but does NOT clean the dictionary itself. The entity-quality bug is
//! recurring (also surfaced in Brain View people-quality and gates CRM entity-ID
//! stability work). It affects more than STT: entity-based retrieval, CRM, and
//! entity-keyed features. These corrector workarounds should be revisited if/when
//! the dictionary is cleaned.

use std::collections::HashSet;
use std::sync::LazyLock;

/// Minimum Jaro-Winkler similarity to consider a correction.
///
/// The original note here claimed "real proper-noun hits sit at 0.97+; false
/// positives cluster 0.85–0.94". Measured against a real session on
/// 2026-08-12, that is not true and cannot be made true by tuning:
///
/// | heard    | corrected to | score  | verdict     |
/// |----------|--------------|--------|-------------|
/// | `alifax` | `Halifax`    | 0.9524 | correct     |
/// | `Chart`  | `chat`       | 0.9533 | **wrong**   |
///
/// The genuine correction scored *lower* than the corruption. Score alone
/// cannot separate them, which is why the real guard is the shape of the
/// correction TARGET — see [`is_proper_noun_shaped`].
const SIMILARITY_THRESHOLD: f64 = 0.95;

/// Minimum token length (chars) to consider for correction.
/// Short words are too risky (many common words are 3-4 chars).
const MIN_TOKEN_LENGTH: usize = 5;

/// A preloaded set of known proper nouns for a voice session.
#[derive(Clone)]
pub struct EntityDictionary {
    entries: Vec<(String, String)>,
}

impl EntityDictionary {
    pub fn new(names: HashSet<String>) -> Self {
        let entries: Vec<(String, String)> = names
            .into_iter()
            .filter(|n| !n.is_empty())
            // Exclude common English words from the dictionary — they should
            // never be correction targets (e.g. "click", "drive", "graph").
            .filter(|n| !is_common_word(n))
            // …and exclude anything that is not shaped like a proper noun. The
            // hand-maintained list above cannot keep up with what the brain
            // harvests: `chat`, `loop` and `date` all reached the dictionary as
            // "entities" and each one corrupted a real word (see
            // `is_proper_noun_shaped`).
            .filter(|n| is_proper_noun_shaped(n))
            .map(|n| {
                let lower = n.to_lowercase();
                (n, lower)
            })
            .collect();
        Self { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Is this entity name shaped like a proper noun, and therefore safe to rewrite
/// a user's spoken word INTO?
///
/// The brain harvests entity names from graph canonicals and annotation
/// display-names, and that harvest includes ordinary lowercase words. In one
/// real session those junk entities produced three corruptions and one genuine
/// fix:
///
/// ```text
///   Chart -> chat     (broke a web-search request for tide charts)
///   loops -> loop
///   dated -> date
///   alifax -> Halifax (the only correct one)
/// ```
///
/// Every corruption pointed at an all-lowercase target; the correct one pointed
/// at a capitalized name. Requiring at least one uppercase character keeps real
/// proper nouns — `Halifax`, `Kinrows`, and inner-capital brands like `iPhone`
/// or `eBay` — while dropping `chat`, `loop` and `date` at the source.
///
/// The asymmetry justifies erring this way: a missed correction costs the agent
/// a slightly odd word it can usually read through, while a wrong one silently
/// rewrites the user's intent and can misdirect an entire turn.
fn is_proper_noun_shaped(name: &str) -> bool {
    name.chars().any(|c| c.is_uppercase())
}

/// Load entity names from the Brain for use as a proper-noun dictionary.
///
/// Sources:
/// 1. Graph neighborhood entities from `brain.recall()` — canonical names
/// 2. Direct SQL on `memory_annotations.who` — display_name values
///
/// Must be called from `spawn_blocking` — uses raw_blocking_handle() internally.
pub fn load_entity_names_blocking(brain: &permagent::brain_handle::SafeBrain) -> HashSet<String> {
    let brain = brain.raw_blocking_handle();
    let mut names = HashSet::new();

    // Source 1: graph neighborhood entities via recall
    if let Ok(result) = brain.recall("", spectral::Visibility::Private) {
        for ent in &result.graph.neighborhood.entities {
            if !ent.canonical.is_empty() {
                names.insert(ent.canonical.clone());
            }
        }
    }

    // Source 2: all distinct display_names from memory_annotations
    if let Ok(conn) = crate::brain_ops::read_only_brain_conn() {
        if let Ok(mut stmt) = conn.prepare("SELECT DISTINCT who FROM memory_annotations") {
            if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
                for who_json in rows.flatten() {
                    if let Ok(refs) = serde_json::from_str::<Vec<serde_json::Value>>(&who_json) {
                        for r in &refs {
                            if let Some(name) = r.get("display_name").and_then(|v| v.as_str()) {
                                let trimmed = name.trim();
                                if !trimmed.is_empty() {
                                    names.insert(trimmed.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    names
}

/// Correct proper nouns in a transcript using fuzzy matching against the entity dictionary.
///
/// Unigram-only: each whitespace-delimited token is checked individually against
/// single-word entity names. Bigram matching was removed — it produced too many
/// false positives on common two-word English phrases (e.g. "the latest" → "The Mesh").
///
/// Returns the corrected transcript. Logs every correction for observability.
pub fn correct_proper_nouns(transcript: &str, dict: &EntityDictionary) -> String {
    if dict.is_empty() || transcript.is_empty() {
        return transcript.to_string();
    }

    let words: Vec<&str> = transcript.split_whitespace().collect();
    if words.is_empty() {
        return transcript.to_string();
    }

    let mut result_words: Vec<String> = Vec::with_capacity(words.len());

    for word in &words {
        if let Some(correction) = try_correct_token(word, dict) {
            tracing::info!(
                target: "permagentd::voice::stt_correct",
                original = %word,
                corrected = %correction.name,
                similarity = correction.score,
                "Proper noun correction"
            );
            result_words.push(correction.name);
        } else {
            result_words.push(word.to_string());
        }
    }

    result_words.join(" ")
}

struct Correction {
    name: String,
    score: f64,
}

/// Try to correct a single token against single-word entity names.
/// Returns None if no confident correction is found.
fn try_correct_token(token: &str, dict: &EntityDictionary) -> Option<Correction> {
    // Strip trailing punctuation for matching (e.g. "Kinrows." → "Kinrows").
    // Apostrophes stay: they are the possessive we must not drop.
    let stripped = token.trim_end_matches(|c: char| c.is_ascii_punctuation() && c != '\'');
    let punct = token.strip_prefix(stripped).unwrap_or("");
    let (core, inflection) = split_possessive(stripped);
    if core.chars().count() < MIN_TOKEN_LENGTH {
        return None;
    }

    if is_common_word(core) {
        return None;
    }

    let token_lower = core.to_lowercase();
    let token_char_count = token_lower.chars().count();

    // Exact match on the core (case-insensitive) — keep the original token
    // so `Pigkeeper's` is not rewritten to `Pigkeeper`.
    if dict.entries.iter().any(|(_, lower)| *lower == token_lower) {
        return None;
    }

    let mut best_score: f64 = 0.0;
    let mut best_name: Option<&str> = None;
    let mut second_best_score: f64 = 0.0;

    for (original, lower) in &dict.entries {
        // Only match single-word entity names
        if lower.contains(' ') {
            continue;
        }

        // Length ratio guard: skip if lengths differ too much
        let entity_char_count = lower.chars().count();
        let ratio = token_char_count.min(entity_char_count) as f64
            / token_char_count.max(entity_char_count) as f64;
        if ratio < 0.6 {
            continue;
        }

        let score = strsim::jaro_winkler(&token_lower, lower);
        if score > best_score {
            second_best_score = best_score;
            best_score = score;
            best_name = Some(original);
        } else if score > second_best_score {
            second_best_score = score;
        }
    }

    if best_score < SIMILARITY_THRESHOLD {
        return None;
    }

    // Ambiguity guard: if two entities are both close, don't guess
    if second_best_score >= SIMILARITY_THRESHOLD && (best_score - second_best_score) < 0.05 {
        tracing::debug!(
            target: "permagentd::voice::stt_correct",
            token = %token,
            best = best_score,
            second = second_best_score,
            "Skipping ambiguous correction"
        );
        return None;
    }

    best_name.map(|name| Correction {
        name: format!("{name}{inflection}{punct}"),
        score: best_score,
    })
}

/// Split a trailing English possessive (`'s` / `'`) off a token that has
/// already had end punctuation stripped. `Pigkeeper's` → (`Pigkeeper`, `'s`).
fn split_possessive(stripped: &str) -> (&str, &str) {
    if let Some(core) = stripped.strip_suffix("'s") {
        return (core, "'s");
    }
    if let Some(core) = stripped.strip_suffix("'S") {
        return (core, "'S");
    }
    if let Some(core) = stripped.strip_suffix('\'') {
        return (core, "'");
    }
    (stripped, "")
}

fn is_common_word(token: &str) -> bool {
    let lower = token.to_lowercase();
    // Check with and without apostrophe (handles contractions like "what's", "don't")
    if COMMON_WORDS.contains(lower.as_str()) {
        return true;
    }
    // Also check the base form before apostrophe
    if let Some(base) = lower.split('\'').next() {
        if base.len() >= 3 && COMMON_WORDS.contains(base) {
            return true;
        }
    }
    false
}

static COMMON_WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        // Common contractions (base forms checked separately via apostrophe stripping)
        "what's",
        "don't",
        "won't",
        "can't",
        "isn't",
        "aren't",
        "wasn't",
        "weren't",
        "hasn't",
        "haven't",
        "hadn't",
        "doesn't",
        "didn't",
        "couldn't",
        "wouldn't",
        "shouldn't",
        "that's",
        "there's",
        "here's",
        "where's",
        "who's",
        "let's",
        "it's",
        "i'm",
        "i've",
        "i'll",
        "i'd",
        "you're",
        "you've",
        "you'll",
        "you'd",
        "he's",
        "he'll",
        "he'd",
        "she's",
        "she'll",
        "she'd",
        "we're",
        "we've",
        "we'll",
        "we'd",
        "they're",
        "they've",
        "they'll",
        "they'd",
        // Common short words (< 5 chars, guarded by MIN_TOKEN_LENGTH for unigrams
        // but included here for explicit is_common_word checks)
        "also",
        "been",
        "call",
        "came",
        "come",
        "deal",
        "done",
        "down",
        "each",
        "even",
        "ever",
        "face",
        "fact",
        "feel",
        "fill",
        "find",
        "form",
        "from",
        "full",
        "gave",
        "give",
        "goes",
        "gone",
        "good",
        "grow",
        "half",
        "hand",
        "hard",
        "have",
        "head",
        "hear",
        "help",
        "here",
        "high",
        "hold",
        "home",
        "hope",
        "idea",
        "into",
        "item",
        "just",
        "keep",
        "kept",
        "kind",
        "knew",
        "know",
        "laid",
        "land",
        "last",
        "late",
        "lead",
        "left",
        "less",
        "life",
        "like",
        "line",
        "list",
        "live",
        "long",
        "look",
        "lord",
        "lose",
        "loss",
        "lost",
        "love",
        "made",
        "main",
        "make",
        "many",
        "mark",
        "mass",
        "mean",
        "meet",
        "mind",
        "mine",
        "miss",
        "mode",
        "more",
        "most",
        "move",
        "much",
        "must",
        "name",
        "near",
        "need",
        "news",
        "next",
        "nice",
        "none",
        "note",
        "once",
        "only",
        "onto",
        "open",
        "over",
        "pack",
        "page",
        "paid",
        "pair",
        "part",
        "pass",
        "past",
        "path",
        "pick",
        "plan",
        "play",
        "plot",
        "plus",
        "pull",
        "push",
        "race",
        "rain",
        "rate",
        "read",
        "real",
        "rely",
        "rest",
        "rich",
        "ride",
        "ring",
        "rise",
        "risk",
        "road",
        "rock",
        "role",
        "roll",
        "room",
        "root",
        "rule",
        "rush",
        "safe",
        "said",
        "sake",
        "sale",
        "same",
        "save",
        "seat",
        "seek",
        "seem",
        "seen",
        "self",
        "sell",
        "send",
        "sent",
        "show",
        "shut",
        "side",
        "sign",
        "site",
        "size",
        "skin",
        "slow",
        "soft",
        "soil",
        "sold",
        "sole",
        "some",
        "song",
        "soon",
        "sort",
        "soul",
        "spot",
        "star",
        "stay",
        "step",
        "stop",
        "such",
        "sure",
        "take",
        "talk",
        "tall",
        "task",
        "team",
        "tell",
        "tend",
        "term",
        "test",
        "text",
        "than",
        "that",
        "them",
        "then",
        "they",
        "thin",
        "this",
        "thus",
        "till",
        "time",
        "tiny",
        "told",
        "tone",
        "took",
        "tool",
        "tops",
        "tour",
        "town",
        "tree",
        "trip",
        "true",
        "turn",
        "type",
        "unit",
        "upon",
        "used",
        "user",
        "vast",
        "very",
        "view",
        "vote",
        "wage",
        "wait",
        "wake",
        "walk",
        "wall",
        "want",
        "warm",
        "warn",
        "wash",
        "wave",
        "ways",
        "weak",
        "wear",
        "week",
        "well",
        "went",
        "were",
        "west",
        "what",
        "when",
        "whom",
        "wide",
        "wife",
        "wild",
        "will",
        "wind",
        "wine",
        "wire",
        "wise",
        "wish",
        "with",
        "wood",
        "word",
        "wore",
        "work",
        "yard",
        "yeah",
        "year",
        "zero",
        "zone",
        // High-frequency words that appear in transcripts and could false-match entities
        "anything",
        "everything",
        "something",
        "nothing",
        "someone",
        "everyone",
        "anyone",
        "whatever",
        "whenever",
        "wherever",
        "however",
        "another",
        "because",
        "between",
        "through",
        "already",
        "against",
        "without",
        "getting",
        "looking",
        "morning",
        "working",
        "talking",
        "telling",
        "running",
        "opening",
        "closing",
        "reading",
        "playing",
        "coming",
        "going",
        "being",
        "having",
        "making",
        "taking",
        "giving",
        "saying",
        "trying",
        "using",
        "doing",
        "seeing",
        "showing",
        "called",
        "latest",
        "design",
        "things",
        "really",
        "pretty",
        "little",
        "people",
        // 5-letter words that collide with names/proper nouns
        "about",
        "above",
        "after",
        "again",
        "along",
        "angel",
        "atlas",
        "baker",
        "banks",
        "being",
        "berry",
        "black",
        "blank",
        "block",
        "bloom",
        "board",
        "brand",
        "break",
        "bring",
        "brook",
        "brown",
        "brush",
        "build",
        "burns",
        "calls",
        "candy",
        "carol",
        "carry",
        "chase",
        "check",
        "chief",
        "china",
        "class",
        "clean",
        "clear",
        "click",
        "cliff",
        "climb",
        "close",
        "cloud",
        "coach",
        "color",
        "coral",
        "could",
        "count",
        "cover",
        "craft",
        "crane",
        "cross",
        "crown",
        "daily",
        "dance",
        "delta",
        "doubt",
        "draft",
        "drain",
        "drawn",
        "dress",
        "drink",
        "drive",
        "early",
        "earth",
        "eight",
        "ember",
        "empty",
        "enter",
        "equal",
        "error",
        "event",
        "every",
        "exact",
        "extra",
        "faith",
        "false",
        "fancy",
        "fetch",
        "field",
        "fight",
        "final",
        "first",
        "fixed",
        "flash",
        "floor",
        "focus",
        "force",
        "forth",
        "found",
        "frame",
        "frank",
        "fresh",
        "front",
        "given",
        "glass",
        "globe",
        "going",
        "grace",
        "grade",
        "grain",
        "grand",
        "grant",
        "graph",
        "grass",
        "great",
        "green",
        "group",
        "grove",
        "guard",
        "guess",
        "guide",
        "happy",
        "haven",
        "heart",
        "heavy",
        "henry",
        "homer",
        "honey",
        "honor",
        "house",
        "human",
        "ideal",
        "image",
        "index",
        "inner",
        "input",
        "issue",
        "ivory",
        "jimmy",
        "judge",
        "juice",
        "known",
        "label",
        "lance",
        "large",
        "later",
        "laugh",
        "layer",
        "learn",
        "leave",
        "level",
        "light",
        "limit",
        "linen",
        "local",
        "lower",
        "lucky",
        "lunar",
        "lunch",
        "major",
        "maker",
        "march",
        "marsh",
        "mason",
        "match",
        "maybe",
        "mercy",
        "metal",
        "might",
        "miles",
        "minor",
        "model",
        "money",
        "month",
        "moral",
        "mount",
        "mouse",
        "movie",
        "music",
        "never",
        "night",
        "noble",
        "noise",
        "north",
        "noted",
        "novel",
        "occur",
        "ocean",
        "offer",
        "often",
        "olive",
        "opera",
        "order",
        "other",
        "ought",
        "outer",
        "owner",
        "paint",
        "panel",
        "paper",
        "party",
        "patch",
        "peace",
        "penny",
        "phase",
        "phone",
        "photo",
        "piano",
        "piece",
        "pilot",
        "pitch",
        "place",
        "plain",
        "plane",
        "plant",
        "plate",
        "plaza",
        "point",
        "pound",
        "power",
        "press",
        "price",
        "pride",
        "prime",
        "print",
        "prior",
        "prize",
        "proof",
        "proud",
        "prove",
        "queen",
        "quest",
        "quick",
        "quiet",
        "quite",
        "quote",
        "radio",
        "raise",
        "range",
        "rapid",
        "ratio",
        "reach",
        "ready",
        "realm",
        "reign",
        "right",
        "river",
        "robin",
        "rocky",
        "roger",
        "roman",
        "rough",
        "round",
        "route",
        "royal",
        "rural",
        "saint",
        "sandy",
        "scale",
        "scene",
        "scope",
        "scout",
        "sense",
        "serve",
        "seven",
        "shall",
        "shape",
        "share",
        "sharp",
        "sheet",
        "shell",
        "shift",
        "shine",
        "shirt",
        "shock",
        "shore",
        "short",
        "shout",
        "sight",
        "since",
        "sixth",
        "skill",
        "sleep",
        "slide",
        "small",
        "smart",
        "smile",
        "smith",
        "smoke",
        "solar",
        "solid",
        "solve",
        "sorry",
        "sound",
        "south",
        "space",
        "spare",
        "speak",
        "speed",
        "spend",
        "spike",
        "split",
        "sport",
        "squad",
        "staff",
        "stage",
        "stake",
        "stand",
        "start",
        "state",
        "steal",
        "steam",
        "steel",
        "steep",
        "stern",
        "stick",
        "still",
        "stock",
        "stone",
        "stood",
        "store",
        "storm",
        "story",
        "strip",
        "stuck",
        "study",
        "stuff",
        "style",
        "sugar",
        "super",
        "surge",
        "sweet",
        "swing",
        "table",
        "taken",
        "taste",
        "teach",
        "terms",
        "thank",
        "their",
        "theme",
        "there",
        "thick",
        "thing",
        "think",
        "third",
        "those",
        "three",
        "throw",
        "tight",
        "tired",
        "title",
        "today",
        "token",
        "topic",
        "total",
        "touch",
        "tower",
        "trace",
        "track",
        "trade",
        "trail",
        "train",
        "trait",
        "treat",
        "trend",
        "trial",
        "tribe",
        "trick",
        "tried",
        "truck",
        "truly",
        "trust",
        "truth",
        "twice",
        "under",
        "union",
        "unite",
        "unity",
        "until",
        "upper",
        "upset",
        "urban",
        "usage",
        "usual",
        "valid",
        "value",
        "video",
        "vigor",
        "visit",
        "vital",
        "vocal",
        "voice",
        "watch",
        "water",
        "wheel",
        "where",
        "which",
        "while",
        "white",
        "whole",
        "whose",
        "woman",
        "women",
        "world",
        "worry",
        "worse",
        "worst",
        "worth",
        "would",
        "write",
        "wrong",
        "young",
        "youth",
        // 6+ letter common words that could fuzzy-match entity names
        "accept",
        "access",
        "across",
        "action",
        "active",
        "actual",
        "almost",
        "always",
        "amount",
        "answer",
        "anyone",
        "appear",
        "around",
        "arrive",
        "aspect",
        "attack",
        "august",
        "author",
        "battle",
        "beauty",
        "became",
        "become",
        "before",
        "behind",
        "belong",
        "beside",
        "better",
        "beyond",
        "bishop",
        "bottom",
        "branch",
        "breath",
        "bridge",
        "bright",
        "broken",
        "budget",
        "burden",
        "bureau",
        "button",
        "bought",
        "camera",
        "cancel",
        "carbon",
        "career",
        "castle",
        "caught",
        "center",
        "chance",
        "change",
        "charge",
        "choose",
        "church",
        "circle",
        "client",
        "closed",
        "coffee",
        "column",
        "combat",
        "coming",
        "common",
        "comply",
        "copper",
        "corner",
        "cotton",
        "couple",
        "course",
        "cousin",
        "create",
        "credit",
        "crisis",
        "custom",
        "damage",
        "danger",
        "dealer",
        "debate",
        "decade",
        "decide",
        "defeat",
        "defend",
        "define",
        "degree",
        "demand",
        "denial",
        "depend",
        "deploy",
        "deputy",
        "derive",
        "desert",
        "design",
        "desire",
        "detail",
        "detect",
        "device",
        "devote",
        "differ",
        "dinner",
        "direct",
        "divide",
        "doctor",
        "dollar",
        "domain",
        "double",
        "driven",
        "during",
        "easily",
        "editor",
        "effect",
        "effort",
        "eighth",
        "either",
        "emerge",
        "empire",
        "enable",
        "ending",
        "energy",
        "engage",
        "engine",
        "enough",
        "ensure",
        "entire",
        "entity",
        "escape",
        "estate",
        "ethnic",
        "evolve",
        "exceed",
        "except",
        "excuse",
        "expand",
        "expect",
        "expert",
        "export",
        "expose",
        "extend",
        "extent",
        "fabric",
        "factor",
        "fairly",
        "family",
        "farmer",
        "father",
        "fellow",
        "female",
        "figure",
        "filter",
        "finger",
        "finish",
        "flight",
        "flower",
        "follow",
        "forget",
        "formal",
        "former",
        "foster",
        "fourth",
        "freely",
        "freeze",
        "friend",
        "frozen",
        "future",
        "garden",
        "gather",
        "gender",
        "gentle",
        "global",
        "golden",
        "ground",
        "growth",
        "guilty",
        "guitar",
        "handle",
        "happen",
        "harbor",
        "hardly",
        "hazard",
        "health",
        "height",
        "hidden",
        "holder",
        "honest",
        "horror",
        "hunger",
        "hunter",
        "ignore",
        "impact",
        "import",
        "impose",
        "income",
        "indeed",
        "infant",
        "inform",
        "injury",
        "insert",
        "inside",
        "insist",
        "insure",
        "intact",
        "intend",
        "intent",
        "invest",
        "island",
        "itself",
        "junior",
        "kindly",
        "knight",
        "labour",
        "ladder",
        "launch",
        "lawyer",
        "layout",
        "leader",
        "league",
        "lender",
        "length",
        "lesson",
        "letter",
        "likely",
        "linear",
        "liquid",
        "listen",
        "little",
        "living",
        "locate",
        "lovely",
        "luxury",
        "mainly",
        "manage",
        "manner",
        "manual",
        "marble",
        "margin",
        "marine",
        "marker",
        "market",
        "master",
        "matter",
        "medium",
        "member",
        "memory",
        "mental",
        "merely",
        "method",
        "middle",
        "mighty",
        "miller",
        "minute",
        "mirror",
        "mobile",
        "modern",
        "modest",
        "moment",
        "mostly",
        "mother",
        "motion",
        "moving",
        "murder",
        "museum",
        "muscle",
        "mutual",
        "myself",
        "namely",
        "narrow",
        "nation",
        "native",
        "nature",
        "nearby",
        "nearly",
        "neatly",
        "nobody",
        "normal",
        "notice",
        "notion",
        "number",
        "object",
        "obtain",
        "occupy",
        "offend",
        "office",
        "online",
        "oppose",
        "option",
        "orange",
        "origin",
        "outfit",
        "output",
        "palace",
        "parade",
        "parent",
        "partly",
        "patent",
        "patrol",
        "patron",
        "people",
        "period",
        "permit",
        "person",
        "phrase",
        "planet",
        "player",
        "please",
        "pledge",
        "plenty",
        "pocket",
        "poison",
        "police",
        "policy",
        "polish",
        "poorly",
        "poster",
        "potato",
        "powder",
        "prayer",
        "prefer",
        "pretty",
        "prison",
        "profit",
        "prompt",
        "proper",
        "proven",
        "public",
        "pursue",
        "rabbit",
        "random",
        "rather",
        "rating",
        "reader",
        "really",
        "reason",
        "recall",
        "recent",
        "record",
        "reduce",
        "reform",
        "refuse",
        "regard",
        "regime",
        "region",
        "relate",
        "relief",
        "remain",
        "remedy",
        "remote",
        "remove",
        "render",
        "rental",
        "repair",
        "repeat",
        "report",
        "rescue",
        "resign",
        "resist",
        "resort",
        "result",
        "retail",
        "retain",
        "retire",
        "return",
        "reveal",
        "review",
        "revolt",
        "reward",
        "rhythm",
        "rubber",
        "ruling",
        "runner",
        "safely",
        "salary",
        "sample",
        "saving",
        "scared",
        "scheme",
        "school",
        "screen",
        "search",
        "season",
        "second",
        "secret",
        "secure",
        "select",
        "senior",
        "series",
        "server",
        "settle",
        "severe",
        "shadow",
        "shield",
        "should",
        "signal",
        "silent",
        "silver",
        "simple",
        "simply",
        "singer",
        "single",
        "sister",
        "slight",
        "slowly",
        "smooth",
        "social",
        "solely",
        "source",
        "speech",
        "sphere",
        "spirit",
        "spread",
        "spring",
        "square",
        "stable",
        "status",
        "steady",
        "stolen",
        "strain",
        "strand",
        "stream",
        "street",
        "stress",
        "strict",
        "strike",
        "string",
        "stroke",
        "strong",
        "struck",
        "stupid",
        "submit",
        "sudden",
        "suffer",
        "summer",
        "summit",
        "supply",
        "surely",
        "survey",
        "switch",
        "symbol",
        "system",
        "tackle",
        "talent",
        "target",
        "temple",
        "tenant",
        "tender",
        "terror",
        "thanks",
        "theory",
        "thirty",
        "though",
        "thread",
        "threat",
        "throne",
        "thrown",
        "ticket",
        "timber",
        "tissue",
        "tongue",
        "toward",
        "travel",
        "treaty",
        "tribal",
        "trophy",
        "tunnel",
        "twelve",
        "twenty",
        "unique",
        "unless",
        "unlike",
        "update",
        "useful",
        "valley",
        "varied",
        "vendor",
        "verbal",
        "verify",
        "viewer",
        "virgin",
        "vision",
        "visual",
        "volume",
        "walker",
        "warmth",
        "wealth",
        "weapon",
        "weekly",
        "weight",
        "wholly",
        "wicked",
        "widely",
        "window",
        "winner",
        "winter",
        "wisdom",
        "within",
        "wonder",
        "wooden",
        "worker",
        "worthy",
        "writer",
        // 7+ letter common words
        "ability",
        "absence",
        "academy",
        "account",
        "achieve",
        "acquire",
        "address",
        "advance",
        "adviser",
        "against",
        "already",
        "analyze",
        "ancient",
        "another",
        "anxiety",
        "anybody",
        "applied",
        "arrange",
        "article",
        "assault",
        "average",
        "backing",
        "balance",
        "banking",
        "barrier",
        "battery",
        "bearing",
        "because",
        "believe",
        "benefit",
        "besides",
        "between",
        "brought",
        "cabinet",
        "capable",
        "capital",
        "capture",
        "careful",
        "central",
        "century",
        "certain",
        "chamber",
        "chapter",
        "charity",
        "charter",
        "chicken",
        "circuit",
        "classic",
        "climate",
        "clothes",
        "cluster",
        "collect",
        "college",
        "comfort",
        "command",
        "comment",
        "company",
        "compare",
        "compete",
        "complex",
        "compose",
        "concern",
        "conduct",
        "confirm",
        "connect",
        "consent",
        "consist",
        "contact",
        "contain",
        "content",
        "context",
        "control",
        "convert",
        "cooking",
        "correct",
        "counter",
        "country",
        "crucial",
        "culture",
        "current",
        "cutting",
        "dealing",
        "declare",
        "decline",
        "default",
        "defence",
        "deliver",
        "deposit",
        "deserve",
        "despite",
        "destroy",
        "develop",
        "digital",
        "disable",
        "disease",
        "display",
        "dispute",
        "diverse",
        "economy",
        "edition",
        "educate",
        "element",
        "embrace",
        "emotion",
        "endless",
        "enforce",
        "enhance",
        "episode",
        "essence",
        "examine",
        "example",
        "exactly",
        "excited",
        "execute",
        "exhibit",
        "expense",
        "explain",
        "exploit",
        "explore",
        "express",
        "extreme",
        "factory",
        "faculty",
        "failure",
        "fashion",
        "feature",
        "federal",
        "feeling",
        "fiction",
        "finally",
        "finance",
        "finding",
        "foreign",
        "formula",
        "fortune",
        "forward",
        "founder",
        "freedom",
        "further",
        "gallery",
        "general",
        "genetic",
        "genuine",
        "getting",
        "glimpse",
        "growing",
        "halfway",
        "harmful",
        "herself",
        "highway",
        "himself",
        "history",
        "holiday",
        "honesty",
        "however",
        "husband",
        "illegal",
        "imagine",
        "imagine",
        "impulse",
        "include",
        "initial",
        "inquiry",
        "inspect",
        "install",
        "instead",
        "interim",
        "involve",
        "journal",
        "justice",
        "keyword",
        "kingdom",
        "kitchen",
        "landing",
        "lasting",
        "learned",
        "leather",
        "liberal",
        "liberty",
        "license",
        "limited",
        "loading",
        "lockout",
        "lottery",
        "machine",
        "mailbox",
        "manager",
        "massive",
        "meaning",
        "measure",
        "medical",
        "meeting",
        "mention",
        "message",
        "million",
        "mineral",
        "minimum",
        "miracle",
        "mission",
        "mixture",
        "monitor",
        "monthly",
        "morning",
        "mystery",
        "natural",
        "neither",
        "network",
        "nothing",
        "nuclear",
        "nursing",
        "obvious",
        "offense",
        "officer",
        "ongoing",
        "opinion",
        "organic",
        "outline",
        "outlook",
        "overall",
        "oversee",
        "package",
        "parking",
        "partial",
        "partner",
        "passage",
        "patient",
        "pattern",
        "payment",
        "penalty",
        "pension",
        "perfect",
        "perform",
        "perhaps",
        "picture",
        "pioneer",
        "plastic",
        "pleased",
        "popular",
        "portion",
        "poverty",
        "predict",
        "premier",
        "premium",
        "prepare",
        "present",
        "prevent",
        "primary",
        "printer",
        "private",
        "problem",
        "proceed",
        "process",
        "produce",
        "product",
        "profile",
        "program",
        "project",
        "promise",
        "promote",
        "protect",
        "protein",
        "protest",
        "provide",
        "publish",
        "purpose",
        "qualify",
        "quarter",
        "quickly",
        "radical",
        "reality",
        "realize",
        "receipt",
        "recover",
        "reflect",
        "regular",
        "related",
        "release",
        "relying",
        "remains",
        "removal",
        "renewal",
        "replace",
        "request",
        "require",
        "resolve",
        "respect",
        "respond",
        "restore",
        "revenue",
        "reverse",
        "routine",
        "running",
        "satisfy",
        "scatter",
        "scholar",
        "science",
        "section",
        "segment",
        "serious",
        "service",
        "session",
        "setting",
        "several",
        "shelter",
        "silence",
        "similar",
        "sincere",
        "sitting",
        "society",
        "somehow",
        "special",
        "speaker",
        "sponsor",
        "squeeze",
        "station",
        "storage",
        "strange",
        "stretch",
        "subject",
        "succeed",
        "success",
        "suggest",
        "summary",
        "support",
        "supreme",
        "surface",
        "surgery",
        "surplus",
        "survive",
        "suspect",
        "sustain",
        "teacher",
        "tension",
        "therapy",
        "thereby",
        "thought",
        "through",
        "tonight",
        "totally",
        "tourism",
        "tourist",
        "trading",
        "traffic",
        "trouble",
        "turning",
        "typical",
        "uniform",
        "unknown",
        "upgrade",
        "utility",
        "variety",
        "vehicle",
        "venture",
        "version",
        "veteran",
        "victory",
        "village",
        "visible",
        "waiting",
        "weather",
        "website",
        "welcome",
        "welfare",
        "western",
        "whether",
        "willing",
        "without",
        "working",
        "writing",
        "written",
    ]
    .into_iter()
    .collect()
});

#[cfg(test)]
mod tests {
    use super::*;

    /// Dictionary that mimics real Brain entities — includes multi-word names
    /// and common-word-adjacent entities that caused false positives in testing.
    fn real_dict() -> EntityDictionary {
        let names: HashSet<String> = [
            "Kinrows",
            "Sharratt",
            "GetLadle",
            "LAUFT",
            "Permagent",
            "The Mesh",
            "WhatsApp",
            "Grocery Saver",
            "Finder",
            "open source",
            "design work",
            // Polluted entries — common words that exist in real Brain dicts.
            // EntityDictionary::new must filter these out.
            "click",
            "Drive",
            "graph",
            "calls",
            "nothing",
            "design",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        EntityDictionary::new(names)
    }

    // ── Positive tests: corrections that SHOULD fire ──

    #[test]
    fn corrects_close_misspelling() {
        let dict = real_dict();
        // "Kinros" → "Kinrows" at J-W ~0.97 (above 0.95 threshold)
        let result = correct_proper_nouns("I was talking about Kinros yesterday", &dict);
        assert!(result.contains("Kinrows"), "got: {}", result);
    }

    #[test]
    fn kinras_below_threshold() {
        let dict = real_dict();
        // "Kinras" vs "Kinrows": J-W ~0.91 — below 0.95 threshold, NOT corrected
        let result = correct_proper_nouns("I mentioned Kinras earlier", &dict);
        assert!(result.contains("Kinras"), "got: {}", result);
    }

    #[test]
    fn common_words_filtered_from_dict() {
        let dict = real_dict();
        assert!(
            !dict.entries.iter().any(|(_, lower)| [
                "click", "drive", "graph", "calls", "nothing", "design"
            ]
            .contains(&lower.as_str())),
            "Common words should be filtered from entity dictionary"
        );
    }

    #[test]
    fn exact_match_untouched() {
        let dict = real_dict();
        assert_eq!(
            correct_proper_nouns("Open Kinrows project", &dict),
            "Open Kinrows project"
        );
    }

    #[test]
    fn case_insensitive_exact_match() {
        let dict = real_dict();
        assert_eq!(
            correct_proper_nouns("open kinrows now", &dict),
            "open kinrows now"
        );
    }

    // ── Negative tests: ordinary speech must pass through UNCHANGED ──

    #[test]
    fn the_latest_unchanged() {
        let dict = real_dict();
        assert_eq!(
            correct_proper_nouns("show me the latest", &dict),
            "show me the latest"
        );
    }

    #[test]
    fn the_deal_unchanged() {
        let dict = real_dict();
        assert_eq!(
            correct_proper_nouns("what is the deal", &dict),
            "what is the deal"
        );
    }

    #[test]
    fn open_the_projects_tab_unchanged() {
        let dict = real_dict();
        assert_eq!(
            correct_proper_nouns("open the projects tab", &dict),
            "open the projects tab"
        );
    }

    #[test]
    fn the_other_unchanged() {
        let dict = real_dict();
        assert_eq!(
            correct_proper_nouns("try the other one", &dict),
            "try the other one"
        );
    }

    #[test]
    fn dont_want_unchanged() {
        let dict = real_dict();
        assert_eq!(
            correct_proper_nouns("I don't want that", &dict),
            "I don't want that"
        );
    }

    #[test]
    fn whats_unchanged() {
        let dict = real_dict();
        // "What's" must NOT become "WhatsApp"
        assert_eq!(
            correct_proper_nouns("What's the status", &dict),
            "What's the status"
        );
    }

    #[test]
    fn anything_unchanged() {
        let dict = real_dict();
        assert_eq!(
            correct_proper_nouns("is there anything else", &dict),
            "is there anything else"
        );
    }

    #[test]
    fn design_rate_unchanged() {
        let dict = real_dict();
        assert_eq!(
            correct_proper_nouns("what is the design rate", &dict),
            "what is the design rate"
        );
    }

    #[test]
    fn what_do_you_think_of_the_layout() {
        let dict = real_dict();
        let input = "what do you think of the layout";
        assert_eq!(correct_proper_nouns(input, &dict), input);
    }

    #[test]
    fn ordinary_speech_unharmed() {
        let dict = real_dict();
        let input = "what is the weather like today in the city";
        assert_eq!(correct_proper_nouns(input, &dict), input);
    }

    // ── Real voice false-positive regression tests (round 2) ──

    #[test]
    fn graham_not_corrected_to_graph() {
        let dict = real_dict();
        // "Graham" is a real name; must NOT become "graph" (which is filtered from dict anyway)
        let result = correct_proper_nouns("ask Graham about it", &dict);
        assert!(result.contains("Graham"), "got: {}", result);
    }

    #[test]
    fn clicked_not_corrected_to_click() {
        let dict = real_dict();
        let result = correct_proper_nouns("I clicked the button", &dict);
        assert!(result.contains("clicked"), "got: {}", result);
    }

    #[test]
    fn drilled_not_corrected_to_drive() {
        let dict = real_dict();
        let result = correct_proper_nouns("we drilled into the data", &dict);
        assert!(result.contains("drilled"), "got: {}", result);
    }

    #[test]
    fn call_off_not_corrected_to_calls() {
        let dict = real_dict();
        let result = correct_proper_nouns("we need to call-off the meeting", &dict);
        assert!(result.contains("call-off"), "got: {}", result);
    }

    // ── Edge cases ──

    #[test]
    fn short_words_never_corrected() {
        let dict = real_dict();
        assert_eq!(
            correct_proper_nouns("I went to the loft", &dict),
            "I went to the loft"
        );
    }

    #[test]
    fn common_words_never_corrected() {
        let dict = real_dict();
        assert_eq!(
            correct_proper_nouns("the project is ready", &dict),
            "the project is ready"
        );
    }

    #[test]
    fn leaves_distant_word_uncorrected() {
        let dict = real_dict();
        let result = correct_proper_nouns("I spoke to Kenner about it", &dict);
        assert!(result.contains("Kenner"), "got: {}", result);
    }

    #[test]
    fn empty_transcript_passes_through() {
        let dict = real_dict();
        assert_eq!(correct_proper_nouns("", &dict), "");
    }

    #[test]
    fn empty_dict_passes_through() {
        let dict = EntityDictionary::new(HashSet::new());
        assert_eq!(
            correct_proper_nouns("talking about Kinrows", &dict),
            "talking about Kinrows"
        );
    }

    /// Regression, 2026-08-12 voice session. Four corrections fired; three
    /// corrupted the user's words. `Chart -> chat` is the one that mattered:
    /// the user said "use your web search tool to look up the Tide Chart",
    /// received "Tide chat", and the agent spent the turn floundering into the
    /// browser and 404s. The same request typed as text worked perfectly,
    /// because text never passes through this corrector.
    #[test]
    fn junk_lowercase_entities_no_longer_corrupt_real_words() {
        // Exactly the entity names the brain had harvested, junk and all.
        let dict = EntityDictionary::new(
            ["chat", "loop", "date", "Halifax", "Kinrows"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );

        for phrase in [
            "look up the Tide Chart for tomorrow",
            "you can run agentic loops to build code",
            "the file is dated yesterday",
        ] {
            assert_eq!(
                correct_proper_nouns(phrase, &dict),
                phrase,
                "must not rewrite ordinary words: {phrase}"
            );
        }

        // …while the one correction that WAS right still happens.
        assert_eq!(
            correct_proper_nouns("the tides for alifax harbor", &dict),
            "the tides for Halifax harbor"
        );
    }

    /// Inner-capital brands are proper nouns even though they start lowercase.
    #[test]
    fn proper_noun_shape_accepts_inner_capitals_rejects_plain_words() {
        for good in ["Halifax", "Kinrows", "iPhone", "eBay", "McDonald"] {
            assert!(is_proper_noun_shaped(good), "should be kept: {good}");
        }
        for junk in ["chat", "loop", "date", "graph", "webhook"] {
            assert!(!is_proper_noun_shaped(junk), "should be dropped: {junk}");
        }
    }

    /// 2026-08-27 kitchen: STT produced `What's the Pigkeeper's name?` and the
    /// corrector rewrote the possessive to `Pigkeeper` (Jaro-Winkler ~0.96
    /// against the Brain entity). A known name in the possessive must keep `'s`.
    #[test]
    fn pigkeepers_possessive_is_not_stripped() {
        let dict = EntityDictionary::new(["Pigkeeper".to_string()].into_iter().collect());
        let input = "What's the Pigkeeper's name?";
        let result = correct_proper_nouns(input, &dict);
        assert!(
            result.contains("Pigkeeper's"),
            "possessive was stripped: {result}"
        );
    }
}
