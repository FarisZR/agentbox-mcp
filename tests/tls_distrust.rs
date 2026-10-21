use std::time::Duration;

use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use webpki::{EndEntityCert, Error, KeyUsage, anchor_from_trusted_cert};

fn verify(leaf: &[u8], intermediate: &[u8]) -> Result<(), Error> {
    // Trust an unrelated, test-only root. This deliberately bypasses bundled-root
    // removal so the test exercises key-level distrust of the intermediate.
    let root = CertificateDer::from(include_bytes!("fixtures/tls/root.der").as_slice());
    let anchors = [anchor_from_trusted_cert(&root).unwrap()];
    let intermediates = [CertificateDer::from(intermediate)];
    let leaf_der = CertificateDer::from(leaf);
    let cert = EndEntityCert::try_from(&leaf_der).unwrap();
    cert.verify_for_usage(
        webpki::ALL_VERIFICATION_ALGS,
        &anchors,
        &intermediates,
        UnixTime::since_unix_epoch(Duration::from_secs(1_792_540_800)),
        KeyUsage::server_auth(),
        None,
        None,
    )?;
    cert.verify_is_valid_for_subject_name(&ServerName::try_from("localhost").unwrap())
}

fn assert_distrusted(result: Result<(), Error>) {
    // Compare the diagnostic so this regression can also be run against the
    // vulnerable verifier, which did not yet define the CertDistrusted variant.
    assert_eq!(
        result.map_err(|error| format!("{error:?}")),
        Err("CertDistrusted".to_string()),
    );
}

#[test]
fn rejects_compromised_intermediate_key() {
    assert_distrusted(verify(
        include_bytes!("fixtures/tls/distrusted-leaf.der"),
        include_bytes!("fixtures/tls/distrusted-intermediate.der"),
    ));
}

#[test]
fn rejects_same_key_reissued_with_another_subject_and_serial() {
    assert_distrusted(verify(
        include_bytes!("fixtures/tls/reissued-leaf.der"),
        include_bytes!("fixtures/tls/reissued-intermediate.der"),
    ));
}

#[test]
fn accepts_ordinary_valid_chain() {
    assert_eq!(
        verify(
            include_bytes!("fixtures/tls/valid-leaf.der"),
            include_bytes!("fixtures/tls/valid-intermediate.der"),
        ),
        Ok(()),
    );
}
