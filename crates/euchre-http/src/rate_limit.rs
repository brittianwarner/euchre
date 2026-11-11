//! D11: in-process per-IP token buckets for the endpoints that mint credentials or
//! create actors, plus the gateway, plus the desktop downloads' byte budget.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::config::{BucketConfig, RateLimitConfig};

/// Which bucket family a request draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteClass {
    /// `POST /api/device/enroll`.
    Enroll,
    /// `POST /api/identity/identify`.
    Identify,
    /// `POST /api/tables`.
    Tables,
    /// `POST /api/table-session/{gameId}`.
    TableSession,
    /// `/rivet/**`.
    Gateway,
    /// `GET /download/files/<file>`: one token per MiB sent.
    Download,
}

/// Buckets are pruned once the map holds this many entries.
const PRUNE_AT: usize = 50_000;

/// A prune runs at most once per this interval, so a map held above
/// [`PRUNE_AT`] costs one O(n) sweep per second, not one per request.
const PRUNE_INTERVAL: Duration = Duration::from_secs(1);

/// Hard cap on live buckets. Past it, a client with no bucket yet draws from the
/// shared [`OVERFLOW_CLIENT`] bucket of its class instead of growing the map.
const MAX_BUCKETS: usize = 100_000;

/// Key shared by every new client once the map is at [`MAX_BUCKETS`].
pub const OVERFLOW_CLIENT: IpAddr = IpAddr::V6(Ipv6Addr::new(0x100, 0, 0, 0, 0, 0, 0, 1));

/// Key for requests with no usable client IP: they share one bucket.
pub const UNKNOWN_CLIENT: IpAddr = IpAddr::V6(Ipv6Addr::UNSPECIFIED);

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

/// The bucket key of a client IP: the address itself for IPv4 (IPv4-mapped IPv6
/// included), the `/64` prefix for IPv6, since one host normally controls a whole
/// `/64`.
pub fn client_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
        }
    }
}

#[derive(Debug)]
struct Buckets {
    map: HashMap<(RouteClass, IpAddr), Bucket>,
    last_prune: Option<Instant>,
}

/// A set of token buckets keyed by `(route class, client key)` (see
/// [`client_key`]).
#[derive(Debug)]
pub struct RateLimiter {
    config: RateLimitConfig,
    buckets: Mutex<Buckets>,
}

impl RateLimiter {
    /// A limiter with `config`.
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            buckets: Mutex::new(Buckets {
                map: HashMap::new(),
                last_prune: None,
            }),
        }
    }

    /// Take one token for `ip`; `false` when the bucket is empty.
    pub fn check(&self, class: RouteClass, ip: IpAddr) -> bool {
        self.check_at(class, ip, Instant::now())
    }

    /// [`RateLimiter::check`] at an explicit instant.
    pub fn check_at(&self, class: RouteClass, ip: IpAddr, now: Instant) -> bool {
        self.take_at(class, ip, 1.0, now).is_ok()
    }

    /// Take `cost` tokens for `ip`. `Err(wait)` when the bucket holds fewer, with
    /// how long until it holds enough; nothing is taken then. A cost above the
    /// bucket's capacity is charged as the capacity, so it can always pass once
    /// the bucket is full.
    pub fn take(&self, class: RouteClass, ip: IpAddr, cost: f64) -> Result<(), Duration> {
        self.take_at(class, ip, cost, Instant::now())
    }

    /// [`RateLimiter::take`] at an explicit instant.
    pub fn take_at(
        &self,
        class: RouteClass,
        ip: IpAddr,
        cost: f64,
        now: Instant,
    ) -> Result<(), Duration> {
        let cfg = class_config(&self.config, class);
        if cfg == BucketConfig::UNLIMITED {
            return Ok(());
        }
        let capacity = f64::from(cfg.capacity);
        let per_sec = f64::from(cfg.per_minute) / 60.0;
        let mut guard = self.buckets.lock();
        let buckets = &mut *guard;
        if buckets.map.len() >= PRUNE_AT
            && buckets
                .last_prune
                .is_none_or(|at| now.saturating_duration_since(at) >= PRUNE_INTERVAL)
        {
            prune(&mut buckets.map, &self.config, now);
            buckets.last_prune = Some(now);
        }
        let mut key = (class, client_key(ip));
        if buckets.map.len() >= MAX_BUCKETS && !buckets.map.contains_key(&key) {
            key = (class, OVERFLOW_CLIENT);
        }
        let bucket = buckets.map.entry(key).or_insert(Bucket {
            tokens: capacity,
            updated: now,
        });
        let elapsed = now.saturating_duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * per_sec).min(capacity);
        bucket.updated = now;
        let cost = cost.clamp(0.0, capacity);
        if bucket.tokens >= cost {
            bucket.tokens -= cost;
            Ok(())
        } else {
            let wait = (cost - bucket.tokens) / per_sec;
            Err(Duration::try_from_secs_f64(wait).unwrap_or(Duration::from_secs(60)))
        }
    }
}

fn class_config(config: &RateLimitConfig, class: RouteClass) -> BucketConfig {
    match class {
        RouteClass::Enroll => config.enroll,
        RouteClass::Identify => config.identify,
        RouteClass::Tables => config.tables,
        RouteClass::TableSession => config.table_session,
        RouteClass::Gateway => config.gateway,
        RouteClass::Download => config.download,
    }
}

/// Drop every bucket that has refilled completely: it carries no state.
fn prune(
    buckets: &mut HashMap<(RouteClass, IpAddr), Bucket>,
    config: &RateLimitConfig,
    now: Instant,
) {
    buckets.retain(|(class, _), bucket| {
        let cfg = class_config(config, *class);
        let per_sec = f64::from(cfg.per_minute) / 60.0;
        let elapsed = now.saturating_duration_since(bucket.updated).as_secs_f64();
        bucket.tokens + elapsed * per_sec < f64::from(cfg.capacity)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter() -> RateLimiter {
        RateLimiter::new(RateLimitConfig {
            enroll: BucketConfig::new(2, 60),
            ..RateLimitConfig::unlimited()
        })
    }

    #[test]
    fn a_bucket_empties_then_refills() {
        let limiter = limiter();
        let ip: IpAddr = [203, 0, 113, 9].into();
        let t0 = Instant::now();
        assert!(limiter.check_at(RouteClass::Enroll, ip, t0));
        assert!(limiter.check_at(RouteClass::Enroll, ip, t0));
        assert!(!limiter.check_at(RouteClass::Enroll, ip, t0));
        // 60 per minute = one per second.
        assert!(limiter.check_at(RouteClass::Enroll, ip, t0 + Duration::from_secs(1)));
        assert!(!limiter.check_at(RouteClass::Enroll, ip, t0 + Duration::from_secs(1)));
    }

    #[test]
    fn buckets_are_per_ip_and_per_class() {
        let limiter = limiter();
        let a: IpAddr = [203, 0, 113, 9].into();
        let b: IpAddr = [203, 0, 113, 10].into();
        let t0 = Instant::now();
        assert!(limiter.check_at(RouteClass::Enroll, a, t0));
        assert!(limiter.check_at(RouteClass::Enroll, a, t0));
        assert!(!limiter.check_at(RouteClass::Enroll, a, t0));
        assert!(limiter.check_at(RouteClass::Enroll, b, t0));
        // Unlimited classes never refuse.
        for _ in 0..1000 {
            assert!(limiter.check_at(RouteClass::Tables, a, t0));
        }
    }

    #[test]
    fn take_charges_a_cost_and_says_how_long_to_wait() {
        let limiter = RateLimiter::new(RateLimitConfig {
            download: BucketConfig::new(100, 60),
            ..RateLimitConfig::unlimited()
        });
        let ip: IpAddr = [203, 0, 113, 9].into();
        let t0 = Instant::now();
        assert_eq!(limiter.take_at(RouteClass::Download, ip, 60.0, t0), Ok(()));
        // 40 left; 50 more is 10 short, at one token a second.
        assert_eq!(
            limiter.take_at(RouteClass::Download, ip, 50.0, t0),
            Err(Duration::from_secs(10))
        );
        // A refusal takes nothing.
        assert_eq!(limiter.take_at(RouteClass::Download, ip, 40.0, t0), Ok(()));
        // A cost over the capacity is charged as the capacity.
        let t1 = t0 + Duration::from_secs(100);
        assert_eq!(limiter.take_at(RouteClass::Download, ip, 1e9, t1), Ok(()));
        assert!(limiter.take_at(RouteClass::Download, ip, 1.0, t1).is_err());
    }

    #[test]
    fn ipv6_clients_share_a_bucket_per_64() -> anyhow::Result<()> {
        let limiter = limiter();
        let t0 = Instant::now();
        let a: IpAddr = "2001:db8:1:2::1".parse()?;
        let b: IpAddr = "2001:db8:1:2:ffff:ffff:ffff:fffe".parse()?;
        let other: IpAddr = "2001:db8:1:3::1".parse()?;
        assert!(limiter.check_at(RouteClass::Enroll, a, t0));
        assert!(limiter.check_at(RouteClass::Enroll, b, t0));
        // Same /64: the bucket is already empty.
        assert!(!limiter.check_at(RouteClass::Enroll, "2001:db8:1:2::99".parse()?, t0));
        assert!(limiter.check_at(RouteClass::Enroll, other, t0));
        // IPv4 keeps per-address keys, IPv4-mapped included.
        assert_eq!(
            client_key("::ffff:203.0.113.9".parse()?),
            "203.0.113.9".parse::<IpAddr>()?
        );
        assert_eq!(
            client_key("203.0.113.9".parse()?),
            "203.0.113.9".parse::<IpAddr>()?
        );
        Ok(())
    }

    #[test]
    fn new_clients_past_the_cap_share_the_overflow_bucket() {
        let limiter = limiter();
        let t0 = Instant::now();
        {
            let mut guard = limiter.buckets.lock();
            for i in 0..MAX_BUCKETS {
                let ip = IpAddr::from(u32::try_from(i).unwrap_or_default().to_be_bytes());
                // Empty buckets: prune keeps them.
                guard.map.insert(
                    (RouteClass::Enroll, ip),
                    Bucket {
                        tokens: 0.0,
                        updated: t0,
                    },
                );
            }
        }
        let fresh_a: IpAddr = [203, 0, 113, 1].into();
        let fresh_b: IpAddr = [203, 0, 113, 2].into();
        assert!(limiter.check_at(RouteClass::Enroll, fresh_a, t0));
        assert!(limiter.check_at(RouteClass::Enroll, fresh_b, t0));
        // Both drew from one overflow bucket of capacity 2.
        assert!(!limiter.check_at(RouteClass::Enroll, [203, 0, 113, 3].into(), t0));
        let guard = limiter.buckets.lock();
        assert_eq!(guard.map.len(), MAX_BUCKETS + 1);
        assert!(
            guard
                .map
                .contains_key(&(RouteClass::Enroll, OVERFLOW_CLIENT))
        );
    }

    #[test]
    fn prune_runs_at_most_once_per_interval() {
        let limiter = limiter();
        let t0 = Instant::now();
        {
            let mut guard = limiter.buckets.lock();
            for i in 0..PRUNE_AT {
                let ip = IpAddr::from(u32::try_from(i).unwrap_or_default().to_be_bytes());
                guard.map.insert(
                    (RouteClass::Enroll, ip),
                    Bucket {
                        tokens: 0.0,
                        updated: t0,
                    },
                );
            }
        }
        assert!(limiter.check_at(RouteClass::Enroll, [203, 0, 113, 1].into(), t0));
        let first = limiter.buckets.lock().last_prune;
        assert_eq!(first, Some(t0));
        assert!(limiter.check_at(
            RouteClass::Enroll,
            [203, 0, 113, 2].into(),
            t0 + Duration::from_millis(10)
        ));
        assert_eq!(limiter.buckets.lock().last_prune, Some(t0));
        // Two seconds on, every empty bucket has refilled: the next sweep drops them.
        assert!(limiter.check_at(
            RouteClass::Enroll,
            [203, 0, 113, 3].into(),
            t0 + Duration::from_secs(2)
        ));
        assert!(limiter.buckets.lock().map.len() < 10);
    }

    #[test]
    fn prune_keeps_only_partial_buckets() {
        let cfg = RateLimitConfig {
            enroll: BucketConfig::new(2, 60),
            ..RateLimitConfig::unlimited()
        };
        let t0 = Instant::now();
        let mut map = HashMap::new();
        map.insert(
            (RouteClass::Enroll, IpAddr::from([1, 1, 1, 1])),
            Bucket {
                tokens: 0.0,
                updated: t0,
            },
        );
        map.insert(
            (RouteClass::Enroll, IpAddr::from([2, 2, 2, 2])),
            Bucket {
                tokens: 2.0,
                updated: t0,
            },
        );
        prune(&mut map, &cfg, t0);
        assert_eq!(map.len(), 1);
    }
}
