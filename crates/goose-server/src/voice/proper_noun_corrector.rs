//! Post-STT proper-noun correction against Brain entity names.
//!
//! After STT produces a transcript, fuzzy-match tokens against a dictionary of
//! known proper nouns (people, projects, products) sourced from the Brain.
//! Replace close matches above a conservative threshold.

use std::collections::HashSet;
use std::sync::LazyLock;

/// Minimum Jaro-Winkler similarity to consider a correction.
const SIMILARITY_THRESHOLD: f64 = 0.85;

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

/// Load entity names from the Brain for use as a proper-noun dictionary.
///
/// Sources:
/// 1. Graph neighborhood entities from `brain.recall()` — canonical names
/// 2. Direct SQL on `memory_annotations.who` — display_name values
///
/// Must be called from `spawn_blocking` (Brain uses block_on internally).
pub fn load_entity_names(brain: &spectral::Brain) -> HashSet<String> {
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
    let mut skip_next = false;

    for i in 0..words.len() {
        if skip_next {
            skip_next = false;
            continue;
        }

        // Try bigram first (for multi-word entity names like "Jesse Sharratt")
        if i + 1 < words.len() {
            let bigram = format!("{} {}", words[i], words[i + 1]);
            if let Some(correction) = try_correct_token(&bigram, dict, true) {
                tracing::info!(
                    target: "permagentd::voice::stt_correct",
                    original = %bigram,
                    corrected = %correction.name,
                    similarity = correction.score,
                    "Proper noun correction (bigram)"
                );
                result_words.push(correction.name);
                skip_next = true;
                continue;
            }
        }

        // Try single token
        let word = words[i];
        if let Some(correction) = try_correct_token(word, dict, false) {
            tracing::info!(
                target: "permagentd::voice::stt_correct",
                original = %word,
                corrected = %correction.name,
                similarity = correction.score,
                "Proper noun correction (unigram)"
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

/// Try to correct a single token or bigram against the entity dictionary.
/// When `bigram` is true, only consider multi-word entity names.
/// Returns None if no confident correction is found.
fn try_correct_token(token: &str, dict: &EntityDictionary, bigram: bool) -> Option<Correction> {
    if token.chars().count() < MIN_TOKEN_LENGTH {
        return None;
    }

    if is_common_word(token) {
        return None;
    }

    let token_lower = token.to_lowercase();
    let token_char_count = token_lower.chars().count();

    // Exact match (case-insensitive) — no correction needed
    if dict.entries.iter().any(|(_, lower)| *lower == token_lower) {
        return None;
    }

    let mut best_score: f64 = 0.0;
    let mut best_name: Option<&str> = None;
    let mut second_best_score: f64 = 0.0;

    for (original, lower) in &dict.entries {
        // Bigrams should only match multi-word entity names
        if bigram && !lower.contains(' ') {
            continue;
        }
        // Single tokens should only match single-word entity names
        if !bigram && lower.contains(' ') {
            continue;
        }

        // Length ratio guard: skip if lengths differ too much
        // (prevents "Kinrose project" matching "Kinrose")
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
        name: name.to_string(),
        score: best_score,
    })
}

fn is_common_word(token: &str) -> bool {
    // For bigrams, check each word individually
    if token.contains(' ') {
        return token.split_whitespace().all(is_common_word);
    }
    let lower = token.to_lowercase();
    COMMON_WORDS.contains(lower.as_str())
}

static COMMON_WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        // 5-letter words that collide with names/proper nouns
        "about", "above", "after", "again", "along", "angel", "atlas", "baker", "banks", "being",
        "berry", "black", "blank", "block", "bloom", "board", "brand", "break", "bring", "brook",
        "brown", "brush", "build", "burns", "candy", "carol", "carry", "chase", "check", "chief",
        "china", "class", "clean", "clear", "cliff", "close", "cloud", "coach", "color", "coral",
        "could", "count", "cover", "craft", "crane", "cross", "crown", "daily", "dance", "delta",
        "doubt", "draft", "drain", "drawn", "dress", "drink", "drive", "early", "earth", "eight",
        "ember", "empty", "enter", "equal", "error", "event", "every", "exact", "extra", "faith",
        "false", "fancy", "fetch", "field", "fight", "final", "first", "fixed", "flash", "floor",
        "focus", "force", "forth", "found", "frame", "frank", "fresh", "front", "given", "glass",
        "globe", "going", "grace", "grade", "grain", "grand", "grant", "grass", "great", "green",
        "group", "grove", "guard", "guess", "guide", "happy", "haven", "heart", "heavy", "henry",
        "homer", "honey", "honor", "house", "human", "ideal", "image", "index", "inner", "input",
        "issue", "ivory", "jimmy", "judge", "juice", "known", "label", "lance", "large", "later",
        "laugh", "layer", "learn", "leave", "level", "light", "limit", "linen", "local", "lower",
        "lucky", "lunar", "lunch", "major", "maker", "march", "marsh", "mason", "match", "maybe",
        "mercy", "metal", "might", "miles", "minor", "model", "money", "month", "moral", "mount",
        "mouse", "movie", "music", "never", "night", "noble", "noise", "north", "noted", "novel",
        "occur", "ocean", "offer", "often", "olive", "opera", "order", "other", "ought", "outer",
        "owner", "paint", "panel", "paper", "party", "patch", "peace", "penny", "phase", "phone",
        "photo", "piano", "piece", "pilot", "pitch", "place", "plain", "plane", "plant", "plate",
        "plaza", "point", "pound", "power", "press", "price", "pride", "prime", "print", "prior",
        "prize", "proof", "proud", "prove", "queen", "quest", "quick", "quiet", "quite", "quote",
        "radio", "raise", "range", "rapid", "ratio", "reach", "ready", "realm", "reign", "right",
        "river", "robin", "rocky", "roger", "roman", "rough", "round", "route", "royal", "rural",
        "saint", "sandy", "scale", "scene", "scope", "scout", "sense", "serve", "seven", "shall",
        "shape", "share", "sharp", "sheet", "shell", "shift", "shine", "shirt", "shock", "shore",
        "short", "shout", "sight", "since", "sixth", "skill", "sleep", "slide", "small", "smart",
        "smile", "smith", "smoke", "solar", "solid", "solve", "sorry", "sound", "south", "space",
        "spare", "speak", "speed", "spend", "spike", "split", "sport", "squad", "staff", "stage",
        "stake", "stand", "start", "state", "steal", "steam", "steel", "steep", "stern", "stick",
        "still", "stock", "stone", "stood", "store", "storm", "story", "strip", "stuck", "study",
        "stuff", "style", "sugar", "super", "surge", "sweet", "swing", "table", "taken", "taste",
        "teach", "terms", "thank", "their", "theme", "there", "thick", "thing", "think", "third",
        "those", "three", "throw", "tight", "tired", "title", "today", "token", "topic", "total",
        "touch", "tower", "trace", "track", "trade", "trail", "train", "trait", "treat", "trend",
        "trial", "tribe", "trick", "tried", "truck", "truly", "trust", "truth", "twice", "under",
        "union", "unite", "unity", "until", "upper", "upset", "urban", "usage", "usual", "valid",
        "value", "video", "vigor", "visit", "vital", "vocal", "voice", "watch", "water", "wheel",
        "where", "which", "while", "white", "whole", "whose", "woman", "women", "world", "worry",
        "worse", "worst", "worth", "would", "write", "wrong", "young", "youth",
        // 6+ letter common words that could fuzzy-match entity names
        "accept", "access", "across", "action", "active", "actual", "almost", "always", "amount",
        "answer", "anyone", "appear", "around", "arrive", "aspect", "attack", "august", "author",
        "battle", "beauty", "became", "become", "before", "behind", "belong", "beside", "better",
        "beyond", "bishop", "bottom", "branch", "breath", "bridge", "bright", "broken", "budget",
        "burden", "bureau", "button", "bought", "camera", "cancel", "carbon", "career", "castle",
        "caught", "center", "chance", "change", "charge", "choose", "church", "circle", "client",
        "closed", "coffee", "column", "combat", "coming", "common", "comply", "copper", "corner",
        "cotton", "couple", "course", "cousin", "create", "credit", "crisis", "custom", "damage",
        "danger", "dealer", "debate", "decade", "decide", "defeat", "defend", "define", "degree",
        "demand", "denial", "depend", "deploy", "deputy", "derive", "desert", "design", "desire",
        "detail", "detect", "device", "devote", "differ", "dinner", "direct", "divide", "doctor",
        "dollar", "domain", "double", "driven", "during", "easily", "editor", "effect", "effort",
        "eighth", "either", "emerge", "empire", "enable", "ending", "energy", "engage", "engine",
        "enough", "ensure", "entire", "entity", "escape", "estate", "ethnic", "evolve", "exceed",
        "except", "excuse", "expand", "expect", "expert", "export", "expose", "extend", "extent",
        "fabric", "factor", "fairly", "family", "farmer", "father", "fellow", "female", "figure",
        "filter", "finger", "finish", "flight", "flower", "follow", "forget", "formal", "former",
        "foster", "fourth", "freely", "freeze", "friend", "frozen", "future", "garden", "gather",
        "gender", "gentle", "global", "golden", "ground", "growth", "guilty", "guitar", "handle",
        "happen", "harbor", "hardly", "hazard", "health", "height", "hidden", "holder", "honest",
        "horror", "hunger", "hunter", "ignore", "impact", "import", "impose", "income", "indeed",
        "infant", "inform", "injury", "insert", "inside", "insist", "insure", "intact", "intend",
        "intent", "invest", "island", "itself", "junior", "kindly", "knight", "labour", "ladder",
        "launch", "lawyer", "layout", "leader", "league", "lender", "length", "lesson", "letter",
        "likely", "linear", "liquid", "listen", "little", "living", "locate", "lovely", "luxury",
        "mainly", "manage", "manner", "manual", "marble", "margin", "marine", "marker", "market",
        "master", "matter", "medium", "member", "memory", "mental", "merely", "method", "middle",
        "mighty", "miller", "minute", "mirror", "mobile", "modern", "modest", "moment", "mostly",
        "mother", "motion", "moving", "murder", "museum", "muscle", "mutual", "myself", "namely",
        "narrow", "nation", "native", "nature", "nearby", "nearly", "neatly", "nobody", "normal",
        "notice", "notion", "number", "object", "obtain", "occupy", "offend", "office", "online",
        "oppose", "option", "orange", "origin", "outfit", "output", "palace", "parade", "parent",
        "partly", "patent", "patrol", "patron", "people", "period", "permit", "person", "phrase",
        "planet", "player", "please", "pledge", "plenty", "pocket", "poison", "police", "policy",
        "polish", "poorly", "poster", "potato", "powder", "prayer", "prefer", "pretty", "prison",
        "profit", "prompt", "proper", "proven", "public", "pursue", "rabbit", "random", "rather",
        "rating", "reader", "really", "reason", "recall", "recent", "record", "reduce", "reform",
        "refuse", "regard", "regime", "region", "relate", "relief", "remain", "remedy", "remote",
        "remove", "render", "rental", "repair", "repeat", "report", "rescue", "resign", "resist",
        "resort", "result", "retail", "retain", "retire", "return", "reveal", "review", "revolt",
        "reward", "rhythm", "rubber", "ruling", "runner", "safely", "salary", "sample", "saving",
        "scared", "scheme", "school", "screen", "search", "season", "second", "secret", "secure",
        "select", "senior", "series", "server", "settle", "severe", "shadow", "shield", "should",
        "signal", "silent", "silver", "simple", "simply", "singer", "single", "sister", "slight",
        "slowly", "smooth", "social", "solely", "source", "speech", "sphere", "spirit", "spread",
        "spring", "square", "stable", "status", "steady", "stolen", "strain", "strand", "stream",
        "street", "stress", "strict", "strike", "string", "stroke", "strong", "struck", "stupid",
        "submit", "sudden", "suffer", "summer", "summit", "supply", "surely", "survey", "switch",
        "symbol", "system", "tackle", "talent", "target", "temple", "tenant", "tender", "terror",
        "thanks", "theory", "thirty", "though", "thread", "threat", "throne", "thrown", "ticket",
        "timber", "tissue", "tongue", "toward", "travel", "treaty", "tribal", "trophy", "tunnel",
        "twelve", "twenty", "unique", "unless", "unlike", "update", "useful", "valley", "varied",
        "vendor", "verbal", "verify", "viewer", "virgin", "vision", "visual", "volume", "walker",
        "warmth", "wealth", "weapon", "weekly", "weight", "wholly", "wicked", "widely", "window",
        "winner", "winter", "wisdom", "within", "wonder", "wooden", "worker", "worthy", "writer",
        // 7+ letter common words
        "ability", "absence", "academy", "account", "achieve", "acquire", "address", "advance",
        "adviser", "against", "already", "analyze", "ancient", "another", "anxiety", "anybody",
        "applied", "arrange", "article", "assault", "average", "backing", "balance", "banking",
        "barrier", "battery", "bearing", "because", "believe", "benefit", "besides", "between",
        "brought", "cabinet", "capable", "capital", "capture", "careful", "central", "century",
        "certain", "chamber", "chapter", "charity", "charter", "chicken", "circuit", "classic",
        "climate", "clothes", "cluster", "collect", "college", "comfort", "command", "comment",
        "company", "compare", "compete", "complex", "compose", "concern", "conduct", "confirm",
        "connect", "consent", "consist", "contact", "contain", "content", "context", "control",
        "convert", "cooking", "correct", "counter", "country", "crucial", "culture", "current",
        "cutting", "dealing", "declare", "decline", "default", "defence", "deliver", "deposit",
        "deserve", "despite", "destroy", "develop", "digital", "disable", "disease", "display",
        "dispute", "diverse", "economy", "edition", "educate", "element", "embrace", "emotion",
        "endless", "enforce", "enhance", "episode", "essence", "examine", "example", "exactly",
        "excited", "execute", "exhibit", "expense", "explain", "exploit", "explore", "express",
        "extreme", "factory", "faculty", "failure", "fashion", "feature", "federal", "feeling",
        "fiction", "finally", "finance", "finding", "foreign", "formula", "fortune", "forward",
        "founder", "freedom", "further", "gallery", "general", "genetic", "genuine", "getting",
        "glimpse", "growing", "halfway", "harmful", "herself", "highway", "himself", "history",
        "holiday", "honesty", "however", "husband", "illegal", "imagine", "imagine", "impulse",
        "include", "initial", "inquiry", "inspect", "install", "instead", "interim", "involve",
        "journal", "justice", "keyword", "kingdom", "kitchen", "landing", "lasting", "learned",
        "leather", "liberal", "liberty", "license", "limited", "loading", "lockout", "lottery",
        "machine", "mailbox", "manager", "massive", "meaning", "measure", "medical", "meeting",
        "mention", "message", "million", "mineral", "minimum", "miracle", "mission", "mixture",
        "monitor", "monthly", "morning", "mystery", "natural", "neither", "network", "nothing",
        "nuclear", "nursing", "obvious", "offense", "officer", "ongoing", "opinion", "organic",
        "outline", "outlook", "overall", "oversee", "package", "parking", "partial", "partner",
        "passage", "patient", "pattern", "payment", "penalty", "pension", "perfect", "perform",
        "perhaps", "picture", "pioneer", "plastic", "pleased", "popular", "portion", "poverty",
        "predict", "premier", "premium", "prepare", "present", "prevent", "primary", "printer",
        "private", "problem", "proceed", "process", "produce", "product", "profile", "program",
        "project", "promise", "promote", "protect", "protein", "protest", "provide", "publish",
        "purpose", "qualify", "quarter", "quickly", "radical", "reality", "realize", "receipt",
        "recover", "reflect", "regular", "related", "release", "relying", "remains", "removal",
        "renewal", "replace", "request", "require", "resolve", "respect", "respond", "restore",
        "revenue", "reverse", "routine", "running", "satisfy", "scatter", "scholar", "science",
        "section", "segment", "serious", "service", "session", "setting", "several", "shelter",
        "silence", "similar", "sincere", "sitting", "society", "somehow", "special", "speaker",
        "sponsor", "squeeze", "station", "storage", "strange", "stretch", "subject", "succeed",
        "success", "suggest", "summary", "support", "supreme", "surface", "surgery", "surplus",
        "survive", "suspect", "sustain", "teacher", "tension", "therapy", "thereby", "thought",
        "through", "tonight", "totally", "tourism", "tourist", "trading", "traffic", "trouble",
        "turning", "typical", "uniform", "unknown", "upgrade", "utility", "variety", "vehicle",
        "venture", "version", "veteran", "victory", "village", "visible", "waiting", "weather",
        "website", "welcome", "welfare", "western", "whether", "willing", "without", "working",
        "writing", "written",
    ]
    .into_iter()
    .collect()
});

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dict() -> EntityDictionary {
        let names: HashSet<String> = ["Kinrose", "Sharratt", "GetLadle", "LAUFT", "Permagent"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        EntityDictionary::new(names)
    }

    #[test]
    fn corrects_close_misspelling() {
        let dict = test_dict();
        // "Kinrows" → "Kinrose" (J-W ~0.90)
        let result = correct_proper_nouns("I was talking about Kinrows yesterday", &dict);
        assert!(
            result.contains("Kinrose"),
            "Expected Kinrose, got: {}",
            result
        );
    }

    #[test]
    fn corrects_kinras_to_kinrose() {
        let dict = test_dict();
        // "Kinras" vs "Kinrose": J-W ~0.91 — above 0.85, corrected
        let result = correct_proper_nouns("I mentioned Kinras earlier", &dict);
        assert!(
            result.contains("Kinrose"),
            "Expected Kinras→Kinrose correction (J-W ~0.91), got: {}",
            result
        );
    }

    #[test]
    fn leaves_distant_word_uncorrected() {
        let dict = test_dict();
        // "Kenner" vs "Kinrose": J-W ~0.74 — well below 0.85
        let result = correct_proper_nouns("I spoke to Kenner about it", &dict);
        assert!(
            result.contains("Kenner"),
            "Should NOT correct Kenner, got: {}",
            result
        );
    }

    #[test]
    fn exact_match_untouched() {
        let dict = test_dict();
        let result = correct_proper_nouns("Open Kinrose project", &dict);
        assert_eq!(result, "Open Kinrose project");
    }

    #[test]
    fn short_words_never_corrected() {
        let dict = test_dict();
        let result = correct_proper_nouns("I went to the loft", &dict);
        assert_eq!(result, "I went to the loft");
    }

    #[test]
    fn common_words_never_corrected() {
        let dict = test_dict();
        // "project" is a common word, should not match "Permagent" or anything
        let result = correct_proper_nouns("the project is ready", &dict);
        assert_eq!(result, "the project is ready");
    }

    #[test]
    fn empty_transcript_passes_through() {
        let dict = test_dict();
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

    #[test]
    fn ordinary_speech_unharmed() {
        let dict = test_dict();
        let input = "what is the weather like today in the city";
        let result = correct_proper_nouns(input, &dict);
        assert_eq!(result, input);
    }

    #[test]
    fn case_insensitive_exact_match() {
        let dict = test_dict();
        // "kinrose" (lowercase) should match "Kinrose" exactly — no correction needed
        let result = correct_proper_nouns("open kinrose now", &dict);
        assert_eq!(result, "open kinrose now");
    }
}
