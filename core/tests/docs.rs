//! The account guide document is generated from the chart; this keeps them identical.
#[test]
fn account_guide_document_is_current() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/user/accounts.md");
    let expected = yy_accountant::chart::markdown();
    if std::env::var_os("YYA_UPDATE_DOCS").is_some() {
        std::fs::write(&path, &expected).unwrap();
    }
    let actual = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        actual == expected,
        "docs/user/accounts.md is stale; run YYA_UPDATE_DOCS=1 cargo test -p yy-accountant --test docs"
    );
}

#[test]
fn tutorial_document_is_current() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/user/tutorial.md");
    let expected = yy_accountant::tutorial::markdown();
    if std::env::var_os("YYA_UPDATE_DOCS").is_some() {
        std::fs::write(&path, &expected).unwrap();
    }
    let actual = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        actual == expected,
        "docs/user/tutorial.md is stale; run YYA_UPDATE_DOCS=1 cargo test -p yy-accountant --test docs"
    );
}

#[test]
fn study_document_is_current() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/user/study.md");
    let expected = yy_accountant::study::markdown();
    if std::env::var_os("YYA_UPDATE_DOCS").is_some() {
        std::fs::write(&path, &expected).unwrap();
    }
    let actual = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        actual == expected,
        "docs/user/study.md is stale; run YYA_UPDATE_DOCS=1 cargo test -p yy-accountant --test docs"
    );
}
