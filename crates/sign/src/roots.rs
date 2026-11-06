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

/// The ICP-Brasil roots, from https://acraiz.icpbrasil.gov.br/credenciadas/RAIZ/: v5, v10,
/// v11 and v12 (RSA, in force), and v2 (expired in 2023; it still vouches for signatures made
/// before then). v6 (Ed448) and v7 (an ITI-specific algorithm) are left out: this crate can't
/// verify their signatures, so they could not complete any chain.
const ICP_BRASIL: &[Root] = &[
    Root { name: "Autoridade Certificadora Raiz Brasileira v2", bytes: include_bytes!("../roots/icp-brasil/ICP-Brasilv2.crt") },
    Root { name: "Autoridade Certificadora Raiz Brasileira v5", bytes: include_bytes!("../roots/icp-brasil/ICP-Brasilv5.crt") },
    Root { name: "Autoridade Certificadora Raiz Brasileira v10", bytes: include_bytes!("../roots/icp-brasil/ICP-Brasilv10.crt") },
    Root { name: "Autoridade Certificadora Raiz Brasileira v11", bytes: include_bytes!("../roots/icp-brasil/ICP-Brasilv11.crt") },
    Root { name: "Autoridade Certificadora Raiz Brasileira v12", bytes: include_bytes!("../roots/icp-brasil/ICP-Brasilv12.crt") },
];

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

    /// A Receita Federal e-CNPJ issuer (AC SAFEWEB RFB v5, under AC Secretaria da Receita
    /// Federal do Brasil v4) chains to a built-in root, Raiz Brasileira v5: with the roots, an
    /// e-CNPJ signature from it is trusted without the user adding anything.
    #[test]
    fn a_receita_federal_issuer_chains_to_a_built_in_root() {
        let issuer = load_certificates(include_bytes!("../tests/data/icp-brasil/ac-safeweb-rfb-v5.pem")).unwrap().remove(0);
        let ac_srf = load_certificates(include_bytes!("../tests/data/icp-brasil/ac-srf-v4.pem")).unwrap().remove(0);
        let roots = builtin();
        let mut pool = vec![ac_srf];
        pool.extend(roots.iter().cloned());
        let chain = crate::x509::build_chain(&issuer, &pool, None);
        let top = chain.last().expect("a chain");
        assert!(roots.iter().any(|r| r.raw == top.raw), "the chain ends at a built-in root: {}", top.display_name());
        assert_eq!(top.display_name(), "Autoridade Certificadora Raiz Brasileira v5");
        // Without the roots it stops at the Receita's own authority.
        let alone = [pool[0].clone()];
        assert!(!crate::x509::build_chain(&issuer, &alone, None).iter().any(|c| roots.iter().any(|r| r.raw == c.raw)));
    }
}
