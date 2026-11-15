//! Test-only filesystem diagnostics. Never format native error messages or paths.

fn format_error(operation: &'static str, error: &std::io::Error) -> String {
    format!(
        "[c1-fs-test] operation={operation} errno={:?}",
        error.raw_os_error()
    )
}

pub(super) fn report(operation: &'static str, error: &std::io::Error) {
    eprintln!("{}", format_error(operation, error));
}

#[cfg(test)]
mod tests {
    use super::format_error;

    #[test]
    fn diagnostics_include_operation_and_errno_without_error_payloads() {
        let native = std::io::Error::from_raw_os_error(13);
        assert_eq!(
            format_error("publish.rename_noreplace", &native),
            "[c1-fs-test] operation=publish.rename_noreplace errno=Some(13)"
        );
        let sensitive = std::io::Error::other("/private/sensitive/model: secret payload");
        assert_eq!(
            format_error("publish.pre_sync", &sensitive),
            "[c1-fs-test] operation=publish.pre_sync errno=None"
        );
    }
}
