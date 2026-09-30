//! The sentences of a public page's `<title>` and description, in each language of the interface (see `email_templates.rs` for the same idea).

use artiferris_domain::package_repository::RepositoryFormat;
use artiferris_domain::user_preferences::Language;

/// What a page is about, for the phrases that name it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Package,
    Image,
}

pub struct SeoText(pub Language);

impl SeoText {
    pub fn explorer_title(&self) -> &'static str {
        match self.0 {
            Language::En => "Explore public packages",
            Language::Fr => "Explorer les paquets publics",
            Language::Es => "Explorar paquetes públicos",
            Language::It => "Esplora i pacchetti pubblici",
            Language::De => "Öffentliche Pakete entdecken",
        }
    }

    pub fn explorer_description(&self) -> &'static str {
        match self.0 {
            Language::En => "Search public npm packages and Docker images hosted on ArtiFerris.",
            Language::Fr => "Recherchez parmi les paquets npm et les images Docker publics hébergés sur ArtiFerris.",
            Language::Es => "Busca entre los paquetes npm y las imágenes Docker públicos alojados en ArtiFerris.",
            Language::It => "Cerca tra i pacchetti npm e le immagini Docker pubblici ospitati su ArtiFerris.",
            Language::De => "Durchsuchen Sie die öffentlichen npm-Pakete und Docker-Images, die auf ArtiFerris gehostet werden.",
        }
    }

    /// The public things of a format, as a plural noun phrase.
    fn public_things(&self, format: RepositoryFormat) -> &'static str {
        match (self.0, format) {
            (Language::En, RepositoryFormat::Npm) => "public npm packages",
            (Language::En, RepositoryFormat::Docker) => "public Docker images",
            (Language::Fr, RepositoryFormat::Npm) => "paquets npm publics",
            (Language::Fr, RepositoryFormat::Docker) => "images Docker publiques",
            (Language::Es, RepositoryFormat::Npm) => "paquetes npm públicos",
            (Language::Es, RepositoryFormat::Docker) => "imágenes Docker públicas",
            (Language::It, RepositoryFormat::Npm) => "pacchetti npm pubblici",
            (Language::It, RepositoryFormat::Docker) => "immagini Docker pubbliche",
            (Language::De, RepositoryFormat::Npm) => "öffentliche npm-Pakete",
            (Language::De, RepositoryFormat::Docker) => "öffentliche Docker-Images",
        }
    }

    pub fn catalog_title(&self, catalog: &str, format: RepositoryFormat) -> String {
        let things = self.public_things(format);
        match self.0 {
            Language::Fr => format!("{catalog} : {things}"),
            _ => format!("{catalog}: {things}"),
        }
    }

    pub fn catalog_description(&self, count: i64, format: RepositoryFormat) -> String {
        let things = self.public_things(format);
        match self.0 {
            Language::En => format!("Search {count} {things} on ArtiFerris. Install from each owner's URL."),
            Language::Fr => format!("Recherchez parmi les {count} {things} sur ArtiFerris. On installe depuis l'URL de chaque propriétaire."),
            Language::Es => format!("Busca entre {count} {things} en ArtiFerris. Se instala desde la URL de cada propietario."),
            Language::It => format!("Cerca tra {count} {things} su ArtiFerris. Si installa dall'URL di ogni proprietario."),
            Language::De => format!("Durchsuchen Sie {count} {things} auf ArtiFerris. Installiert wird über die URL des jeweiligen Eigentümers."),
        }
    }

    /// `1 package`, `3 packages`.
    pub fn count(&self, count: i64, kind: Kind) -> String {
        let (one, other) = match (self.0, kind) {
            (Language::En, Kind::Package) => ("package", "packages"),
            (Language::En, Kind::Image) => ("image", "images"),
            (Language::Fr, Kind::Package) => ("paquet", "paquets"),
            (Language::Fr, Kind::Image) => ("image", "images"),
            (Language::Es, Kind::Package) => ("paquete", "paquetes"),
            (Language::Es, Kind::Image) => ("imagen", "imágenes"),
            (Language::It, Kind::Package) => ("pacchetto", "pacchetti"),
            (Language::It, Kind::Image) => ("immagine", "immagini"),
            (Language::De, Kind::Package) => ("Paket", "Pakete"),
            (Language::De, Kind::Image) => ("Image", "Images"),
        };
        format!("{count} {}", if count == 1 { one } else { other })
    }

    pub fn owner_title(&self, owner: &str) -> String {
        match self.0 {
            Language::En => format!("{owner}: public packages and images"),
            Language::Fr => format!("{owner} : paquets et images publics"),
            Language::Es => format!("{owner}: paquetes e imágenes públicos"),
            Language::It => format!("{owner}: pacchetti e immagini pubblici"),
            Language::De => format!("{owner}: öffentliche Pakete und Images"),
        }
    }

    pub fn owner_description(&self, owner: &str, packages: i64, images: i64) -> String {
        let (packages, images) = (self.count(packages, Kind::Package), self.count(images, Kind::Image));
        match self.0 {
            Language::En => format!("{owner} publishes {packages} and {images} on ArtiFerris."),
            Language::Fr => format!("{owner} publie {packages} et {images} sur ArtiFerris."),
            Language::Es => format!("{owner} publica {packages} y {images} en ArtiFerris."),
            Language::It => format!("{owner} pubblica {packages} e {images} su ArtiFerris."),
            Language::De => format!("{owner} veröffentlicht {packages} und {images} auf ArtiFerris."),
        }
    }

    fn format_name(format: RepositoryFormat) -> &'static str {
        match format {
            RepositoryFormat::Npm => "npm",
            RepositoryFormat::Docker => "Docker",
        }
    }

    pub fn repository_title(&self, owner: &str, repository: &str, format: RepositoryFormat) -> String {
        let what = Self::format_name(format);
        match self.0 {
            Language::En => format!("{owner} / {repository}: public {what} repository"),
            Language::Fr => format!("{owner} / {repository} : dépôt {what} public"),
            Language::Es => format!("{owner} / {repository}: repositorio {what} público"),
            Language::It => format!("{owner} / {repository}: repository {what} pubblico"),
            Language::De => format!("{owner} / {repository}: öffentliches {what}-Repository"),
        }
    }

    pub fn repository_description(&self, owner: &str, repository: &str, format: RepositoryFormat) -> String {
        let what = Self::format_name(format);
        match self.0 {
            Language::En => format!("Public {what} repository \"{repository}\" by {owner} on ArtiFerris."),
            Language::Fr => format!("Dépôt {what} public « {repository} » de {owner} sur ArtiFerris."),
            Language::Es => format!("Repositorio {what} público «{repository}» de {owner} en ArtiFerris."),
            Language::It => format!("Repository {what} pubblico «{repository}» di {owner} su ArtiFerris."),
            Language::De => format!("Öffentliches {what}-Repository „{repository}“ von {owner} auf ArtiFerris."),
        }
    }

    pub fn package_title(&self, name: &str, owner: &str, format: RepositoryFormat) -> String {
        match (self.0, format) {
            (Language::En, RepositoryFormat::Npm) => format!("{name}: npm package by {owner}"),
            (Language::En, RepositoryFormat::Docker) => format!("{name}: Docker image by {owner}"),
            (Language::Fr, RepositoryFormat::Npm) => format!("{name} : paquet npm de {owner}"),
            (Language::Fr, RepositoryFormat::Docker) => format!("{name} : image Docker de {owner}"),
            (Language::Es, RepositoryFormat::Npm) => format!("{name}: paquete npm de {owner}"),
            (Language::Es, RepositoryFormat::Docker) => format!("{name}: imagen Docker de {owner}"),
            (Language::It, RepositoryFormat::Npm) => format!("{name}: pacchetto npm di {owner}"),
            (Language::It, RepositoryFormat::Docker) => format!("{name}: immagine Docker di {owner}"),
            (Language::De, RepositoryFormat::Npm) => format!("{name}: npm-Paket von {owner}"),
            (Language::De, RepositoryFormat::Docker) => format!("{name}: Docker-Image von {owner}"),
        }
    }

    /// `version` is what follows the name, like ` (1.2.3)`, or nothing.
    pub fn package_description(&self, name: &str, version: &str, owner: &str, format: RepositoryFormat) -> String {
        match (self.0, format) {
            (Language::En, RepositoryFormat::Npm) => format!("npm package {name}{version} published by {owner} on ArtiFerris."),
            (Language::En, RepositoryFormat::Docker) => format!("Docker image {name}{version} published by {owner} on ArtiFerris."),
            (Language::Fr, RepositoryFormat::Npm) => format!("Paquet npm {name}{version} publié par {owner} sur ArtiFerris."),
            (Language::Fr, RepositoryFormat::Docker) => format!("Image Docker {name}{version} publiée par {owner} sur ArtiFerris."),
            (Language::Es, RepositoryFormat::Npm) => format!("Paquete npm {name}{version} publicado por {owner} en ArtiFerris."),
            (Language::Es, RepositoryFormat::Docker) => format!("Imagen Docker {name}{version} publicada por {owner} en ArtiFerris."),
            (Language::It, RepositoryFormat::Npm) => format!("Pacchetto npm {name}{version} pubblicato da {owner} su ArtiFerris."),
            (Language::It, RepositoryFormat::Docker) => format!("Immagine Docker {name}{version} pubblicata da {owner} su ArtiFerris."),
            (Language::De, RepositoryFormat::Npm) => format!("npm-Paket {name}{version}, veröffentlicht von {owner} auf ArtiFerris."),
            (Language::De, RepositoryFormat::Docker) => format!("Docker-Image {name}{version}, veröffentlicht von {owner} auf ArtiFerris."),
        }
    }
}

#[cfg(test)]
mod tests {
    use artiferris_domain::user_preferences::SUPPORTED_LANGUAGES;

    use super::*;

    #[test]
    fn every_language_writes_every_sentence_and_they_differ_from_one_language_to_the_next() {
        let sentences = |language: Language| {
            let text = SeoText(language);
            let mut all = vec![text.explorer_title().to_string(), text.explorer_description().to_string(), text.owner_title("o"), text.owner_description("o", 2, 2)];
            for format in [RepositoryFormat::Npm, RepositoryFormat::Docker] {
                all.extend([
                    text.catalog_title("c", format),
                    text.catalog_description(7, format),
                    text.repository_title("o", "r", format),
                    text.repository_description("o", "r", format),
                    text.package_title("n", "o", format),
                    text.package_description("n", " (1.0.0)", "o", format),
                ]);
            }
            all
        };
        for language in SUPPORTED_LANGUAGES {
            for other in SUPPORTED_LANGUAGES.into_iter().filter(|other| *other != language) {
                let (mine, theirs) = (sentences(language), sentences(other));
                let same: Vec<_> = mine.iter().zip(&theirs).filter(|(a, b)| a == b).collect();
                assert!(same.is_empty(), "{language:?} and {other:?} share {same:?}");
            }
        }
    }

    #[test]
    fn the_values_land_in_the_sentences() {
        for language in SUPPORTED_LANGUAGES {
            let text = SeoText(language);
            let description = text.package_description("left-pad", " (1.2.3)", "alice", RepositoryFormat::Npm);
            assert!(description.contains("left-pad (1.2.3)") && description.contains("alice"), "{description}");
            assert!(text.catalog_description(12, RepositoryFormat::Docker).contains("12 "), "{language:?}");
            assert!(text.repository_description("alice", "lib", RepositoryFormat::Npm).contains("lib"), "{language:?}");
        }
    }

    #[test]
    fn one_is_singular_and_the_rest_plural() {
        assert_eq!(SeoText(Language::En).count(1, Kind::Package), "1 package");
        assert_eq!(SeoText(Language::En).count(3, Kind::Image), "3 images");
        assert_eq!(SeoText(Language::Es).count(2, Kind::Image), "2 imágenes");
    }
}
