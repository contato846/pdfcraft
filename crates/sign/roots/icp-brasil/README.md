# ICP-Brasil root certificates

Public trust anchors of the Brazilian public-key infrastructure (ICP-Brasil), published by the
Instituto Nacional de Tecnologia da Informação (ITI) for distribution at
https://acraiz.icpbrasil.gov.br/credenciadas/RAIZ/ and embedded here byte for byte, so that
signatures made with e-CPF and e-CNPJ certificates validate (`crates/sign/src/roots.rs`).

| File | Root | SHA-256 fingerprint |
|---|---|---|
| ICP-Brasilv2.crt | Autoridade Certificadora Raiz Brasileira v2 (expired 2023-06-21) | FB:47:D9:2A:99:09:FD:4F:A9:BE:C0:27:37:54:3E:1F:35:14:CE:D7:47:40:7A:8D:9C:FA:39:7B:09:15:06:7C |
| ICP-Brasilv5.crt | Autoridade Certificadora Raiz Brasileira v5 | CA:A5:3F:C6:09:1C:69:51:88:7C:97:6E:37:8F:6E:F8:9A:A6:37:7C:55:D9:7B:64:75:42:2B:71:ED:7E:9B:17 |
| ICP-Brasilv10.crt | Autoridade Certificadora Raiz Brasileira v10 | 6E:0B:FF:06:9A:26:99:4C:15:DE:2C:48:88:CC:54:AF:84:88:2E:54:95:B7:FB:F6:6B:E9:CC:FF:EC:74:89:F6 |
| ICP-Brasilv11.crt | Autoridade Certificadora Raiz Brasileira v11 | 14:06:71:00:58:18:0F:A4:08:1A:AB:3F:24:6F:17:02:42:9C:55:2A:11:FA:31:43:B8:4C:88:CB:3A:B8:E5:E7 |
| ICP-Brasilv12.crt | Autoridade Certificadora Raiz Brasileira v12 | D8:47:8E:37:CE:19:C6:90:CF:65:73:81:E6:8F:E6:00:E4:E1:A0:42:53:68:30:F0:68:47:E0:3E:55:4C:4B:01 |

`intermediates.pem` holds the intermediate certification authorities ITI publishes at
https://acraiz.icpbrasil.gov.br/credenciadas/CertificadosAC-ICP-Brasil/ACcompactado.zip
(downloaded 2026-11-11, archive SHA-256
8617ea8c102c7c8e21f08e28b4ff836a4dbcdaa46fcb097c814c779c1833d910): every certificate of the
archive that is not self-signed and chains to one of the roots above, re-encoded as PEM and
concatenated in file-name order (173 certificates). They are not trust anchors: they only
complete the chain of a signature that embeds the signer's certificate without its issuers.

The test data in `crates/sign/tests/data/icp-brasil/` are two public intermediate authorities
(AC SAFEWEB RFB v5 and AC Secretaria da Receita Federal do Brasil v4), as any e-CNPJ signature
embeds them. Certificates are public data with no copyright claim; they carry no licence.
