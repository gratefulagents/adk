use ipnet::IpNet;
use std::{
    net::{IpAddr, SocketAddr},
    sync::OnceLock,
    time::Duration,
};
use url::{Host, Url};

pub(crate) fn public_ip(address: IpAddr) -> bool {
    let address = match address {
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(address),
        _ => address,
    };
    if address.is_loopback() || address.is_unspecified() || address.is_multicast() {
        return false;
    }
    if let IpAddr::V4(ip) = address
        && (ip.is_private() || ip.is_link_local() || ip.is_broadcast())
    {
        return false;
    }
    static BLOCKED: OnceLock<Vec<IpNet>> = OnceLock::new();
    !BLOCKED
        .get_or_init(|| {
            [
                "0.0.0.0/8",
                "100.64.0.0/10",
                "127.0.0.0/8",
                "169.254.0.0/16",
                "198.18.0.0/15",
                "100.100.100.200/32",
                "192.0.0.192/29",
                "::/128",
                "::1/128",
                "::ffff:0:0/96",
                "64:ff9b::/96",
                "64:ff9b:1::/48",
                "100::/64",
                "2001:db8::/32",
                "fc00::/7",
                "fe80::/10",
                "fec0::/10",
                "ff00::/8",
            ]
            .into_iter()
            .map(|prefix| prefix.parse().expect("static IP prefix"))
            .collect()
        })
        .iter()
        .any(|prefix| prefix.contains(&address))
}

pub(crate) fn parse(raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    let (scheme, rest) = raw
        .split_once(':')
        .ok_or("url must start with http:// or https://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err("url must start with http:// or https://".into());
    }
    let rest = rest.strip_prefix("//").ok_or("url host is required")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    if authority.is_empty() {
        return Err("url host is required".into());
    }
    if authority.contains('\\') || raw.chars().any(|character| character.is_ascii_control()) {
        return Err("invalid url: invalid control character or hostname".into());
    }
    let url = Url::parse(raw).map_err(|e| format!("invalid url: {e}"))?;
    if authority.contains('@') || !url.username().is_empty() || url.password().is_some() {
        return Err("url must not contain embedded credentials".into());
    }
    if url.host().is_none() {
        return Err("url host is required".into());
    }
    Ok(url)
}

pub(crate) async fn resolve(
    raw: &str,
    allow_private: bool,
) -> Result<(Url, Vec<SocketAddr>), String> {
    let url = parse(raw)?;
    let host = url.host().ok_or("url host is required")?;
    let port = url.port_or_known_default().expect("HTTP default port");
    let addresses = match host {
        Host::Ipv4(ip) => vec![SocketAddr::new(ip.into(), port)],
        Host::Ipv6(ip) => vec![SocketAddr::new(ip.into(), port)],
        Host::Domain(host) => {
            let lower = host.strip_suffix('.').unwrap_or(host).to_lowercase();
            if !allow_private
                && (lower == "localhost"
                    || lower.ends_with(".localhost")
                    || lower.ends_with(".local")
                    || lower == "metadata.google.internal")
            {
                return Err(format!("url host {host:?} is private or local"));
            }
            let addresses = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::net::lookup_host((host, port)),
            )
            .await
            .map_err(|_| format!("resolve url host {host:?}: timed out"))?
            .map_err(|e| format!("resolve url host {host:?}: {e}"))?
            .collect::<Vec<_>>();
            if addresses.is_empty() {
                return Err(format!("resolve url host {host:?}: no addresses"));
            }
            addresses
        }
    };
    validate_addresses(&url, &addresses, allow_private)?;
    Ok((url, addresses))
}

fn validate_addresses(
    url: &Url,
    addresses: &[SocketAddr],
    allow_private: bool,
) -> Result<(), String> {
    if !allow_private {
        for address in addresses {
            if !public_ip(address.ip()) {
                return Err(format!(
                    "url host {:?} resolves to private or local address {}",
                    url.host_str().unwrap_or_default(),
                    address.ip()
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn client(
    url: &Url,
    addresses: &[SocketAddr],
) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(0)
        .resolve_to_addrs(url.host_str().expect("validated host"), addresses)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdk_public_and_private_address_classes() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.1.2",
            "192.168.2.3",
            "169.254.169.254",
            "100.100.100.200",
            "100.64.1.1",
            "198.19.0.1",
            "192.0.0.195",
            "0.1.2.3",
            "255.255.255.255",
            "224.1.2.3",
            "::1",
            "::ffff:127.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b:1::1",
            "100::1",
            "2001:db8::1",
            "fd00::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
        ] {
            assert!(!public_ip(address.parse().unwrap()), "{address}");
        }
        for address in [
            "8.8.8.8",
            "1.1.1.1",
            "192.0.2.1",
            "198.51.100.1",
            "203.0.113.1",
            "::ffff:8.8.8.8",
            "2606:4700:4700::1111",
        ] {
            assert!(public_ip(address.parse().unwrap()), "{address}");
        }
    }
    #[tokio::test]
    async fn url_credentials_local_aliases_and_schemes_are_rejected() {
        for url in [
            "file:///etc/passwd",
            "ftp://example.com",
            "http://@example.com",
            "http://user:pass@example.com",
            "http://localhost/",
            "http://a.localhost/",
            "http://a.local/",
            "http://metadata.google.internal/",
            "http://127.0.0.1/",
            "http://[::1]/",
            "http://0x7f000001/",
        ] {
            assert!(resolve(url, false).await.is_err(), "{url}");
        }
        assert!(resolve("http://127.0.0.1/", true).await.is_ok());
        assert!(resolve("http://user:pass@127.0.0.1/", true).await.is_err());
    }
}

// URL serialization is observable in Browser results and command arguments.
// WHATWG serialization would add '/', normalize ports/host case and dot segments.
pub(crate) fn go_url(raw: &str) -> Result<String, String> {
    let _ = parse(raw)?;
    let (scheme, rest) = raw.trim().split_once(':').expect("validated scheme");
    let rest = rest.strip_prefix("//").expect("validated authority");
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let tail = &rest[end..];
    let (tail, fragment) = tail
        .split_once('#')
        .map_or((tail, None), |(a, b)| (a, Some(b)));
    let (path, query) = tail
        .split_once('?')
        .map_or((tail, None), |(a, b)| (a, Some(b)));
    let authority = url_decode(authority)?;
    let mut output = format!(
        "{}://{}{}",
        scheme.to_ascii_lowercase(),
        url_escape(&authority, 0),
        escaped_component(path, 1)?
    );
    if let Some(query) = query {
        output.push('?');
        output.push_str(query);
    }
    if let Some(fragment) = fragment.filter(|f| !f.is_empty()) {
        output.push('#');
        output.push_str(&escaped_component(fragment, 2)?);
    }
    Ok(output)
}
fn url_decode(value: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut input = value.as_bytes().iter().copied();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let a = input.next().and_then(|b| (b as char).to_digit(16));
            let b = input.next().and_then(|b| (b as char).to_digit(16));
            match (a, b) {
                (Some(a), Some(b)) => bytes.push((a * 16 + b) as u8),
                _ => return Err("invalid url: invalid URL escape".into()),
            }
        } else {
            bytes.push(byte);
        }
    }
    Ok(bytes)
}
fn escape_byte(byte: u8, mode: u8) -> bool {
    if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
        return false;
    }
    if mode == 0 {
        return !b"!$&'()*+,;=:[]<>\"".contains(&byte);
    }
    if b"$&+,/:;=@".contains(&byte) || (mode == 2 && b"?!()*".contains(&byte)) {
        return false;
    }
    true
}
fn url_escape(bytes: &[u8], mode: u8) -> String {
    let mut output = String::new();
    for &byte in bytes {
        if escape_byte(byte, mode) {
            output.push_str(&format!("%{byte:02X}"));
        } else {
            output.push(byte as char);
        }
    }
    output
}
fn escaped_component(value: &str, mode: u8) -> Result<String, String> {
    let bytes = url_decode(value)?;
    if value
        .bytes()
        .all(|byte| b"!$&'()*+,;=:@[]%".contains(&byte) || !escape_byte(byte, mode))
    {
        Ok(value.into())
    } else {
        Ok(url_escape(&bytes, mode))
    }
}

#[cfg(test)]
mod dial_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[test]
    fn mixed_dns_answers_are_rejected_before_dial() {
        let url = Url::parse("http://public.example/").unwrap();
        for addresses in [
            vec!["8.8.8.8:80", "127.0.0.1:80"],
            vec!["127.0.0.1:80", "8.8.8.8:80"],
            vec!["8.8.8.8:80", "[::ffff:169.254.169.254]:80"],
        ] {
            let addresses = addresses
                .into_iter()
                .map(|a| a.parse().unwrap())
                .collect::<Vec<_>>();
            assert!(validate_addresses(&url, &addresses, false).is_err());
            assert!(validate_addresses(&url, &addresses, true).is_ok());
        }
    }
    #[tokio::test]
    async fn client_dials_only_pinned_addresses_without_dns_fallback() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let url = Url::parse(&format!("http://sdk-pinned.invalid:{}/", address.port())).unwrap();
        let request = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let count = stream.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..count]);
            assert!(request.contains("host: sdk-pinned.invalid:"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\npinned",
                )
                .await
                .unwrap();
        });
        let response = client(&url, &[address])
            .unwrap()
            .get(url)
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "pinned");
        request.await.unwrap();
    }
}
