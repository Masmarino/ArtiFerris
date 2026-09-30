use artiferris_domain::organization::Organization;
use artiferris_domain::package_repository::PackageRepositorySummary;
use artiferris_domain::public_catalog::OwnerKind;

/// Where a repository is served from, which decides the install URL: a personal repository under `/u/<user>/<repo>` on
/// the main host, the public organization's repositories directly on the main host, any other organization on its own
/// subdomain.
pub struct RepositoryLocation {
    pub owner_kind: OwnerKind,
    /// The username for a personal owner, the organization slug otherwise.
    pub owner_slug: String,
    pub is_public_organization: bool,
    pub repository_name: String,
}

impl RepositoryLocation {
    pub fn of(repository: &PackageRepositorySummary, owner: &Organization) -> Self {
        // A personal namespace's organization is named after its user.
        let (owner_kind, owner_slug) = if owner.is_personal { (OwnerKind::Personal, owner.display_name.clone()) } else { (OwnerKind::Organization, owner.slug.as_str().to_string()) };
        Self { owner_kind, owner_slug, is_public_organization: owner.is_public, repository_name: repository.name.clone() }
    }

    /// The registry URL to hand to `npm install --registry`, with a trailing slash.
    pub fn registry_url(&self, public_url: &str, base_domain: &str) -> String {
        let (scheme, host, path) = self.resolve(public_url, base_domain);
        format!("{scheme}://{host}/npm/{path}/")
    }

    /// The image reference to pull, without scheme or tag.
    pub fn image_reference(&self, image: &str, public_url: &str, base_domain: &str) -> String {
        let (_, host, path) = self.resolve(public_url, base_domain);
        format!("{host}/{path}/{image}")
    }

    fn resolve<'a>(&self, public_url: &'a str, base_domain: &str) -> (&'a str, String, String) {
        let (scheme, authority) = public_url.split_once("://").map_or(("http", public_url), |(scheme, rest)| (scheme, rest.split('/').next().unwrap_or(rest)));
        let (host, path) = match self.owner_kind {
            OwnerKind::Personal => (authority.to_string(), format!("u/{}/{}", self.owner_slug, self.repository_name)),
            OwnerKind::Organization if self.is_public_organization => (authority.to_string(), self.repository_name.clone()),
            OwnerKind::Organization => {
                let port = authority.rsplit_once(':').filter(|(_, port)| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit())).map(|(_, port)| port);
                (format!("{}.{base_domain}{}", self.owner_slug, port.map(|p| format!(":{p}")).unwrap_or_default()), self.repository_name.clone())
            }
        };
        (scheme, host, path)
    }
}
