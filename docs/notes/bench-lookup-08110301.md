# Similarity scorer lookup

- **Definition:** `crates/goose/src/cards.rs`
- **Duplicate threshold:** `DUPLICATE_DICE_THRESHOLD`
- **Algorithm:** `title_similarity` normalizes each title into a set of distinguishing tokens (removing retry suffixes, stopwords, and interchangeable production verbs) and returns their Sørensen–Dice coefficient, `2|A∩B| / (|A|+|B|)`.
