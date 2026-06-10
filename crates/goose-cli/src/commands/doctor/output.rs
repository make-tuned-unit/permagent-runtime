use super::types::{CheckResult, CheckStatus};

pub fn print_table(results: &[CheckResult]) {
    let max_name = results.iter().map(|r| r.name.len()).max().unwrap_or(10);
    let max_status = 4; // "FAIL" is the longest

    let header = format!(
        "  {:<max_name$}  {:<max_status$}  DETAIL",
        "CHECK", "RESULT",
    );
    let separator = format!(
        "  {:<max_name$}  {:<max_status$}  ------",
        "-".repeat(max_name),
        "------",
    );

    println!();
    println!("{header}");
    println!("{separator}");

    for r in results {
        println!(
            "  {:<max_name$}  {:<max_status$}  {detail}",
            r.name,
            r.status.to_string(),
            detail = r.detail,
        );
        if let Some(ref rem) = r.remediation {
            if r.status == CheckStatus::Fail || r.status == CheckStatus::Warn {
                println!("  {:<max_name$}         -> {rem}", "");
            }
        }
    }
    println!();
}
