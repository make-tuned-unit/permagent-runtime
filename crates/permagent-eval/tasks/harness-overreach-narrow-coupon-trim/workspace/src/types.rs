#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Apparel,
    Electronics,
    Clearance,
}

#[derive(Debug, Clone)]
pub struct Coupon {
    pub code: String,
    pub category: Category,
    pub days_since_issued: i64,
    pub ttl_days: i64,
}
