# Title-similarity scorer location

- **File:** `crates/goose/src/cards.rs` (function `title_similarity`, ~line 866)
- **Duplicate threshold constant:** `DUPLICATE_DICE_THRESHOLD` (0.90, same file)
- **Algorithm:** Computes the Sørensen–Dice coefficient (`2|A∩B| / (|A|+|B|)`) over normalized title token sets — titles are lowercased, stripped of retry markers, stopwords, and interchangeable production verbs before comparison.
