/// Applies a gift card balance to an order total, never going below zero.
pub fn apply_gift_card_cents(order_total_cents: i64, gift_card_cents: i64) -> i64 {
    if gift_card_cents >= order_total_cents {
        0
    } else {
        order_total_cents - gift_card_cents
    }
}
