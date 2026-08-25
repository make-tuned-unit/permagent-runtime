/// Returns how many units are available to sell right now.
pub fn available_quantity(total: u32, reserved: u32) -> u32 {
    total - reserved
}

pub fn is_low_stock(available: u32, threshold: u32) -> bool {
    available <= threshold
}
