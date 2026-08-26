use lpt_lib::lintian::*;

#[test]
fn parses_severity_lines() {
    let r = parse(
        "E: eza: bad-version\nW: eza: no-homepage\nI: eza: no-php\n",
        "",
    );
    assert_eq!(r.errors, 1);
    assert_eq!(r.warnings, 1);
    assert_eq!(r.info, 1);
    assert_eq!(r.lines.len(), 3);
}

#[test]
fn parse_ignores_non_severity_output() {
    let r = parse(
        "lintian check v2.114.0\n\nRunning checks...\n",
        "N: weird line",
    );
    assert_eq!(r.errors, 0);
    assert_eq!(r.warnings, 0);
    assert_eq!(r.info, 0);
}

#[test]
fn fail_rules() {
    let clean = LintianReport::default();
    assert!(!should_fail(&clean, false));

    let err = LintianReport {
        errors: 1,
        ..Default::default()
    };
    assert!(should_fail(&err, false));
    assert!(should_fail(&err, true));

    let warn = LintianReport {
        warnings: 1,
        ..Default::default()
    };
    assert!(!should_fail(&warn, false));
    assert!(should_fail(&warn, true));
}
