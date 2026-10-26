//! Root certificates trusted out of the box, kept apart from the user's own trusted list:
//! they are never saved with it, and Preferences can turn them off as a whole.
//!
//! Today this is the Brazilian public-key infrastructure (ICP-Brasil), whose roots are not in
//! the Adobe Approved Trust List: without them every e-CPF and e-CNPJ signature reads as
//! "validity unknown". The certificates are public (published by ITI, the Brazilian root
//! authority) and are embedded byte for byte as downloaded, one file per root, in `roots/`.

use crate::x509::{Certificate, load_certificates};

/// One built-in root: where it comes from and its bytes (DER or PEM).
struct Root {
    #[allow(dead_code)]
    name: &'static str,
    bytes: &'static [u8],
}

/// The ICP-Brasil roots in use. Empty until the certificate files are added to `roots/`.
const ICP_BRASIL: &[Root] = &[];

/// The built-in roots, parsed. A file that fails to parse is skipped (the test below keeps
/// that from happening silently).
pub fn builtin() -> Vec<Certificate> {
    parse(ICP_BRASIL.iter().map(|r| r.bytes))
}

fn parse<'a>(files: impl Iterator<Item = &'a [u8]>) -> Vec<Certificate> {
    files.filter_map(|b| load_certificates(b).ok()).flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_root_parses_and_is_self_signed() {
        for r in ICP_BRASIL {
            let certs = parse(std::iter::once(r.bytes));
            assert_eq!(certs.len(), 1, "{} parses to one certificate", r.name);
            assert!(certs[0].is_self_signed(), "{} is a root", r.name);
        }
        assert_eq!(builtin().len(), ICP_BRASIL.len());
    }
}
