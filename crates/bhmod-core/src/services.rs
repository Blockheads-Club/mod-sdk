//! The service registry: mods publish `name@version` (declared in their `mod.toml` `[provides]`) and other mods call
//! them by name and a semver requirement. The highest compatible version wins; on a tie the provider loaded first.

use semver::{Version, VersionReq};

use crate::types::{ApiError, ServiceError};

/// Most services one mod may provide.
pub const MAX_PER_MOD: usize = 64;

/// One registered provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    /// The providing mod (slot index = load order).
    pub owner: usize,
    /// Service name.
    pub name: String,
    /// Service version.
    pub version: Version,
}

/// All providers.
#[derive(Debug, Default)]
pub struct ServiceRegistry {
    providers: Vec<Provider>,
}

impl ServiceRegistry {
    /// Registers `owner` as provider of `name@version`. `declared` are the `(name, version)` pairs of the mod's
    /// `[provides]`; the registration must match one of them. Registering the same name again replaces it.
    pub fn provide(
        &mut self,
        owner: usize,
        name: &str,
        version: &str,
        declared: &[(String, String)],
    ) -> Result<(), ApiError> {
        let v = Version::parse(version)
            .map_err(|_| ApiError::InvalidArgument(format!("service version {version:?} is not semver")))?;
        if !declared.iter().any(|(n, dv)| n == name && Version::parse(dv).is_ok_and(|d| d == v)) {
            return Err(ApiError::InvalidArgument(format!("{name}@{version} is not declared in mod.toml [provides]")));
        }
        self.providers.retain(|p| !(p.owner == owner && p.name == name));
        if self.providers.iter().filter(|p| p.owner == owner).count() >= MAX_PER_MOD {
            return Err(ApiError::LimitExceeded("services per mod".into()));
        }
        self.providers.push(Provider { owner, name: name.to_string(), version: v });
        Ok(())
    }

    /// The best provider of `name` matching `req`.
    pub fn lookup(&self, name: &str, req: &str) -> Result<&Provider, ServiceError> {
        let req = VersionReq::parse(req)
            .map_err(|_| ServiceError::BadValue(format!("{req:?} is not a semver requirement")))?;
        let mut named = self.providers.iter().filter(|p| p.name == name).peekable();
        if named.peek().is_none() {
            return Err(ServiceError::NotFound);
        }
        named
            .filter(|p| req.matches(&p.version))
            .min_by(|a, b| b.version.cmp(&a.version).then(a.owner.cmp(&b.owner)))
            .ok_or(ServiceError::VersionMismatch)
    }

    /// Removes everything `owner` provides.
    pub fn remove_owner(&mut self, owner: usize) {
        self.providers.retain(|p| p.owner != owner);
    }

    /// Every provider in registration order.
    pub fn all(&self) -> &[Provider] {
        &self.providers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn provide_rules() {
        let mut r = ServiceRegistry::default();
        let d = decl(&[("economy", "1.2.0")]);
        assert!(matches!(r.provide(0, "economy", "x", &d), Err(ApiError::InvalidArgument(m)) if m.contains("semver")));
        assert!(
            matches!(r.provide(0, "economy", "1.3.0", &d), Err(ApiError::InvalidArgument(m)) if m.contains("[provides]"))
        );
        assert!(r.provide(0, "bank", "1.2.0", &d).is_err());
        r.provide(0, "economy", "1.2.0", &d).unwrap();
        r.provide(0, "economy", "1.2.0", &d).unwrap();
        assert_eq!(r.all().len(), 1);
        r.remove_owner(0);
        assert!(r.all().is_empty());
    }

    #[test]
    fn lookup_picks_highest_then_first_loaded() {
        let mut r = ServiceRegistry::default();
        r.provide(2, "eco", "1.5.0", &decl(&[("eco", "1.5.0")])).unwrap();
        r.provide(1, "eco", "1.5.0", &decl(&[("eco", "1.5.0")])).unwrap();
        r.provide(0, "eco", "1.1.0", &decl(&[("eco", "1.1.0")])).unwrap();
        r.provide(3, "eco", "2.0.0", &decl(&[("eco", "2.0.0")])).unwrap();
        assert_eq!(r.lookup("eco", "^1").unwrap().owner, 1);
        assert_eq!(r.lookup("eco", "^1.0, <1.2").unwrap().owner, 0);
        assert_eq!(r.lookup("eco", "*").unwrap().owner, 3);
        assert_eq!(r.lookup("eco", "^3"), Err(ServiceError::VersionMismatch));
        assert_eq!(r.lookup("bank", "^1"), Err(ServiceError::NotFound));
        assert!(matches!(r.lookup("eco", "not a req"), Err(ServiceError::BadValue(_))));
    }

    #[test]
    fn per_mod_limit() {
        let mut r = ServiceRegistry::default();
        let names: Vec<(String, String)> = (0..=MAX_PER_MOD).map(|i| (format!("s{i}"), "1.0.0".to_string())).collect();
        for (n, v) in &names[..MAX_PER_MOD] {
            r.provide(0, n, v, &names).unwrap();
        }
        let (n, v) = &names[MAX_PER_MOD];
        assert_eq!(r.provide(0, n, v, &names), Err(ApiError::LimitExceeded("services per mod".into())));
    }
}
