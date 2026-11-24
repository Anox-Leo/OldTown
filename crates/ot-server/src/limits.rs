use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::HeaderMap;
use ot_proto::UserId;
use tokio::time::Instant;

const MAX_TRACKED: usize = 65_536;

#[derive(Debug, Clone)]
pub struct LimitsConfig {
    pub max_unauth_per_ip: usize,
    pub max_unauth_total: usize,
    pub failure_burst: u32,
    pub failure_refill: Duration,
    pub trusted_proxies: Vec<IpAddr>,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_unauth_per_ip: 16,
            max_unauth_total: 256,
            failure_burst: 30,
            failure_refill: Duration::from_secs(10),
            trusted_proxies: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    Busy,
    Failures,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Key(IpAddr);

impl Key {
    fn of(ip: IpAddr) -> Self {
        match canonical(ip) {
            IpAddr::V6(v6) => {
                let s = v6.segments();
                Self(IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0)))
            }
            v4 => Self(v4),
        }
    }

    const OVERFLOW: Self = Self(IpAddr::V6(Ipv6Addr::UNSPECIFIED));
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    at: Instant,
}

#[derive(Debug, Default)]
struct State {
    unauth: HashMap<Key, usize>,
    unauth_total: usize,
    failures: HashMap<Key, Bucket>,
}

#[derive(Debug)]
pub struct Limits {
    config: LimitsConfig,
    state: Mutex<State>,
}

impl Limits {
    pub fn new(config: LimitsConfig) -> Arc<Self> {
        Arc::new(Self { config, state: Mutex::default() })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn client_ip(&self, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
        let peer = canonical(peer.ip());
        if !self.trusted(peer) {
            return peer;
        }
        let forwarded = headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .collect::<Vec<_>>();
        forwarded
            .iter()
            .rev()
            .filter_map(|s| s.trim().parse::<IpAddr>().ok().map(canonical))
            .find(|ip| !self.trusted(*ip))
            .unwrap_or(peer)
    }

    fn trusted(&self, ip: IpAddr) -> bool {
        self.config.trusted_proxies.iter().any(|t| canonical(*t) == ip)
    }

    pub fn admit(self: &Arc<Self>, ip: IpAddr) -> Result<Permit, Refused> {
        let key = Key::of(ip);
        let mut state = self.state();
        if self.exhausted(&mut state, key) {
            return Err(Refused::Failures);
        }
        let per_ip = state.unauth.get(&key).copied().unwrap_or(0);
        if state.unauth_total >= self.config.max_unauth_total
            || per_ip >= self.config.max_unauth_per_ip
        {
            return Err(Refused::Busy);
        }
        state.unauth.insert(key, per_ip + 1);
        state.unauth_total += 1;
        Ok(Permit { limits: self.clone(), key })
    }

    pub fn blocked(&self, ip: IpAddr) -> bool {
        let mut state = self.state();
        self.exhausted(&mut state, Key::of(ip))
    }

    pub fn failed(&self, ip: IpAddr) {
        let mut key = Key::of(ip);
        let mut state = self.state();
        if !state.failures.contains_key(&key) && state.failures.len() >= MAX_TRACKED {
            self.prune_locked(&mut state);
            if state.failures.len() >= MAX_TRACKED {
                key = Key::OVERFLOW;
            }
        }
        let burst = f64::from(self.config.failure_burst);
        let bucket = state
            .failures
            .entry(key)
            .or_insert_with(|| Bucket { tokens: burst, at: Instant::now() });
        self.refill(bucket);
        bucket.tokens = (bucket.tokens - 1.0).max(0.0);
    }

    pub fn prune(&self) {
        let mut state = self.state();
        self.prune_locked(&mut state);
    }

    fn prune_locked(&self, state: &mut State) {
        let burst = f64::from(self.config.failure_burst);
        state.failures.retain(|_, b| {
            self.refill(b);
            b.tokens < burst
        });
    }

    fn exhausted(&self, state: &mut State, key: Key) -> bool {
        let Some(bucket) = state.failures.get_mut(&key) else { return false };
        self.refill(bucket);
        bucket.tokens < 1.0
    }

    fn refill(&self, bucket: &mut Bucket) {
        let now = Instant::now();
        let gained = now.duration_since(bucket.at).as_secs_f64()
            / self.config.failure_refill.as_secs_f64().max(f64::EPSILON);
        bucket.tokens = (bucket.tokens + gained).min(f64::from(self.config.failure_burst));
        bucket.at = now;
    }
}

#[derive(Debug)]
pub struct SendRate {
    per_minute: u32,
    buckets: Mutex<HashMap<UserId, Bucket>>,
}

impl SendRate {
    pub fn new(per_minute: u32) -> Self {
        Self { per_minute, buckets: Mutex::default() }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<UserId, Bucket>> {
        self.buckets.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn refill(&self, bucket: &mut Bucket) {
        let now = Instant::now();
        let full = f64::from(self.per_minute);
        let gained = now.duration_since(bucket.at).as_secs_f64() * full / 60.0;
        bucket.tokens = (bucket.tokens + gained).min(full);
        bucket.at = now;
    }

    pub fn take(&self, user: &UserId) -> bool {
        let full = f64::from(self.per_minute);
        let mut buckets = self.lock();
        let bucket = buckets
            .entry(user.clone())
            .or_insert_with(|| Bucket { tokens: full, at: Instant::now() });
        self.refill(bucket);
        if bucket.tokens < 1.0 {
            return false;
        }
        bucket.tokens -= 1.0;
        true
    }

    pub fn prune(&self) {
        let full = f64::from(self.per_minute);
        let mut buckets = self.lock();
        buckets.retain(|_, b| {
            self.refill(b);
            b.tokens < full
        });
    }
}

#[derive(Debug)]
pub struct Permit {
    limits: Arc<Limits>,
    key: Key,
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = self.limits.state();
        state.unauth_total = state.unauth_total.saturating_sub(1);
        if let Some(n) = state.unauth.get_mut(&self.key) {
            *n -= 1;
            if *n == 0 {
                state.unauth.remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn peer(s: &str) -> SocketAddr {
        SocketAddr::new(ip(s), 4242)
    }

    fn limits(config: LimitsConfig) -> Arc<Limits> {
        Limits::new(config)
    }

    #[test]
    fn unauthenticated_connections_are_bounded_per_ip_and_in_total() {
        let l = limits(LimitsConfig {
            max_unauth_per_ip: 2,
            max_unauth_total: 3,
            ..LimitsConfig::default()
        });
        let a1 = l.admit(ip("192.0.2.1")).unwrap();
        let _a2 = l.admit(ip("192.0.2.1")).unwrap();
        assert_eq!(l.admit(ip("192.0.2.1")).unwrap_err(), Refused::Busy);
        let _b1 = l.admit(ip("192.0.2.2")).unwrap();
        assert_eq!(l.admit(ip("192.0.2.3")).unwrap_err(), Refused::Busy, "total atteint");
        drop(a1);
        let _a3 = l.admit(ip("192.0.2.1")).unwrap();
        assert!(l.state().unauth.len() <= 2);
    }

    #[test]
    fn permits_are_released_and_forgotten() {
        let l = limits(LimitsConfig::default());
        drop(l.admit(ip("192.0.2.1")).unwrap());
        let state = l.state();
        assert_eq!(state.unauth_total, 0);
        assert!(state.unauth.is_empty());
    }

    #[test]
    fn ipv6_is_counted_per_64_prefix_and_mapped_ipv4_as_ipv4() {
        let l = limits(LimitsConfig { max_unauth_per_ip: 1, ..LimitsConfig::default() });
        let _p = l.admit(ip("2001:db8:1:2::1")).unwrap();
        assert_eq!(l.admit(ip("2001:db8:1:2::ffff")).unwrap_err(), Refused::Busy);
        let _q = l.admit(ip("2001:db8:1:3::1")).unwrap();
        let _r = l.admit(ip("192.0.2.9")).unwrap();
        assert_eq!(l.admit(ip("::ffff:192.0.2.9")).unwrap_err(), Refused::Busy);
    }

    #[tokio::test(start_paused = true)]
    async fn failures_block_then_recover() {
        let l = limits(LimitsConfig {
            failure_burst: 3,
            failure_refill: Duration::from_secs(10),
            ..LimitsConfig::default()
        });
        let bad = ip("198.51.100.7");
        for _ in 0..3 {
            assert!(!l.blocked(bad));
            l.failed(bad);
        }
        assert!(l.blocked(bad));
        assert_eq!(l.admit(bad).unwrap_err(), Refused::Failures);
        assert!(!l.blocked(ip("198.51.100.8")), "les autres IP ne sont pas touchées");

        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(!l.blocked(bad), "un échec toléré regagné");
        l.failed(bad);
        assert!(l.blocked(bad));

        tokio::time::advance(Duration::from_secs(30)).await;
        l.prune();
        assert!(l.state().failures.is_empty(), "IP revenue à son plein : oubliée");
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_retry_never_blocks() {
        let l = limits(LimitsConfig::default());
        let home = ip("203.0.113.5");
        for _ in 0..(2 * 3600 / 25) {
            l.failed(home);
            assert!(!l.blocked(home));
            tokio::time::advance(Duration::from_secs(25)).await;
        }
    }

    #[test]
    fn forwarded_for_is_read_only_from_a_trusted_proxy() {
        let l = limits(LimitsConfig {
            trusted_proxies: vec![ip("127.0.0.1"), ip("10.0.0.2")],
            ..LimitsConfig::default()
        });
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.1, 203.0.113.9"));

        assert_eq!(l.client_ip(peer("192.0.2.1"), &h), ip("192.0.2.1"), "pas un frontal");
        assert_eq!(l.client_ip(peer("127.0.0.1"), &h), ip("203.0.113.9"), "dernière ajoutée");
        assert_eq!(l.client_ip(peer("::ffff:127.0.0.1"), &h), ip("203.0.113.9"));

        h.append("x-forwarded-for", HeaderValue::from_static("10.0.0.2"));
        assert_eq!(l.client_ip(peer("127.0.0.1"), &h), ip("203.0.113.9"), "frontaux sautés");

        let mut junk = HeaderMap::new();
        junk.insert("x-forwarded-for", HeaderValue::from_static("pas une ip"));
        assert_eq!(l.client_ip(peer("127.0.0.1"), &junk), ip("127.0.0.1"));
        assert_eq!(l.client_ip(peer("127.0.0.1"), &HeaderMap::new()), ip("127.0.0.1"));
    }

    #[tokio::test(start_paused = true)]
    async fn send_rate_is_per_user_and_refills() {
        let rate = SendRate::new(3);
        let (a, b) = (UserId::generate(), UserId::generate());
        for _ in 0..3 {
            assert!(rate.take(&a));
        }
        assert!(!rate.take(&a), "seau vide");
        assert!(rate.take(&b), "une autre personne n'est pas touchée");
        tokio::time::advance(Duration::from_secs(20)).await;
        assert!(rate.take(&a), "un envoi regagné en 20 s");
        assert!(!rate.take(&a));
        tokio::time::advance(Duration::from_secs(120)).await;
        rate.prune();
        assert!(rate.lock().is_empty(), "seaux pleins oubliés");
    }

    #[test]
    fn without_trusted_proxy_the_header_is_ignored() {
        let l = limits(LimitsConfig::default());
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.1"));
        assert_eq!(l.client_ip(peer("127.0.0.1"), &h), ip("127.0.0.1"));
    }
}
