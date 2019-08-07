use acme_client::Directory;

struct CertificateError {
    message: String,
}

impl std::error::Error for CertificateError {
    fn description(&self) -> &str { self.message.as_str() }
    fn cause(&self) -> Option<&dyn std::error::Error> { None }
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> { None }
}

impl std::fmt::Display for CertificateError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "An Error Occurred, Please Try Again!")
    }
}

impl std::fmt::Debug for CertificateError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{{ file: {}, line: {} }}", file!(), line!())
    }
}

impl CertificateError {
    fn new(message: String) -> Self {
        CertificateError { message }
    }
}

impl std::convert::From<acme_client::error::Error> for CertificateError {
    fn from(e: acme_client::error::Error) -> Self {
        return CertificateError::new(e.to_string());
    }
}

struct CertificateRequest<'a> {
    domain: &'a str,
    email: &'a str,
}

impl<'a> CertificateRequest<'a> {
    fn new(email: &'a str, domain: &'a str) -> Self {
        return CertificateRequest { domain, email };
    }

    fn sign(self: &Self) -> Result<(), CertificateError> {
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