struct CertificateRequest<'a> {
    domains: &'a [&'a str]
}

impl<'a> CertificateRequest<'a> {
    fn new(domains: &'a [&'a str]) -> Self {
        return CertificateRequest { domains };
    }



}