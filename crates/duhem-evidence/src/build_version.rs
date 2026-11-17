pub(crate) fn format_build_version(package_version: &str, commit: Option<&str>) -> String {
    match commit {
        Some(commit) => format!("{package_version}+{commit}"),
        None => package_version.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::format_build_version;

    #[test]
    fn formats_build_version_with_commit() {
        assert_eq!(
            format_build_version("0.9.0", Some("abc1234")),
            "0.9.0+abc1234"
        );
    }

    #[test]
    fn formats_build_version_without_commit() {
        assert_eq!(format_build_version("0.9.0", None), "0.9.0");
    }
}
