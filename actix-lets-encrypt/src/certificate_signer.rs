use acme_client::Directory;

struct CertificateRequest<'a> {
    domain: &'a str,
    email: &'a str,
}

impl<'a> CertificateRequest<'a> {
    fn new(email: &'a str, domain: &'a str) -> Self {
        return CertificateRequest { domain, email };
    }

    fn sign(self: &self) -> Result<(), std::io::Error> {
        let directory = Directory::lets_encrypt()?;
        let account = directory.account_registration()
            .email(self.email)
            .register()?;
        let authorization = account.authorization(self.domain)?;

        let http_challenge = authorization.get_http_challenge().ok_or("HTTP challenge failed")?;
        http_challenge.save_key_authorization("/var/www")?;
        http_challenge.validate()?;

        let cert = account.certificate_signer(&[self.domain]).sign_certificate()?;
        cert.save_signed_certificate("certificate.pem")?;
        cert.save_private_key("certificate.key")?;

        Ok(())
    }
}