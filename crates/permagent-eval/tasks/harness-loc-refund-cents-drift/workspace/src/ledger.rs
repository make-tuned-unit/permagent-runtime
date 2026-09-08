#[derive(Debug, Default)]
pub struct Ledger {
    pub entries: Vec<i64>,
}

impl Ledger {
    pub fn record(&mut self, cents: i64) {
        self.entries.push(cents);
    }

    pub fn total(&self) -> i64 {
        self.entries.iter().sum()
    }
}
