//! Subject/text/HTML builders for outbound notifications, sharing one HTML shell. Styles are inline and layout uses `<table>` — email clients don't reliably support stylesheets or
//! `flex`/`grid`. The logo is referenced as `cid:{LOGO_CID}`, not a regular URL, since `SmtpEmailSender` embeds the image bytes under that same id.

use artiferris_domain::user_preferences::Language;

pub struct EmailContent {
    pub subject: String,
    pub text: String,
    pub html: String,
}

/// `SmtpEmailSender` attaches the actual image bytes tagged with this same id.
pub const LOGO_CID: &str = "artiferris-logo";

const PRIMARY: &str = "#0d1ed3";
const TEXT_PRIMARY: &str = "#1f2937";
const TEXT_SECONDARY: &str = "#6b7280";

fn shell(language: Language, preheader: &str, body_html: &str) -> String {
    let lang = language.as_str();
    let footer = words(language).footer;
    format!(
        r#"<!doctype html>
<html lang="{lang}">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>ArtiFerris</title>
  </head>
  <body style="margin:0; padding:0; background-color:#f4f5f7; font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;">
    <span style="display:none; font-size:1px; color:#f4f5f7; line-height:1px; max-height:0; max-width:0; opacity:0; overflow:hidden;">{preheader}</span>
    <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background-color:#f4f5f7; padding:32px 16px;">
      <tr>
        <td align="center">
          <table role="presentation" width="100%" style="max-width:480px; background-color:#ffffff; border-radius:12px; overflow:hidden; box-shadow:0 1px 3px rgba(0,0,0,0.1);" cellpadding="0" cellspacing="0">
            <tr>
              <td style="padding:32px 32px 24px; text-align:center; border-bottom:1px solid #e5e7eb;">
                <img src="cid:{LOGO_CID}" alt="ArtiFerris" width="220" style="display:block; width:220px; max-width:100%; height:auto; margin:0 auto;" />
              </td>
            </tr>
            <tr>
              <td style="padding:32px; font-size:14px; line-height:1.6; color:{TEXT_PRIMARY};">
                {body_html}
              </td>
            </tr>
            <tr>
              <td style="padding:20px 32px; background-color:#f9fafb; text-align:center;">
                <p style="margin:0; font-size:12px; color:{TEXT_SECONDARY};">{footer}</p>
              </td>
            </tr>
          </table>
        </td>
      </tr>
    </table>
  </body>
</html>"#
    )
}

fn button(href: &str, label: &str) -> String {
    format!(
        r#"<table role="presentation" cellpadding="0" cellspacing="0" style="margin:24px 0;"><tr><td style="border-radius:8px; background-color:{PRIMARY};"><a href="{href}" style="display:inline-block; padding:12px 24px; font-size:14px; font-weight:600; color:#ffffff; text-decoration:none;">{label}</a></td></tr></table>"#
    )
}


/// The second factor an e-mail tells the user was added, named in the reader's language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrolledMethod {
    AuthenticatorApp,
    Passkey,
}

/// Every sentence of the e-mails, per language. `{username}` and `{method}` are replaced; the `*_html` ones may carry `<strong>`.
struct Words {
    hello: &'static str,
    footer: &'static str,
    method_authenticator_app: &'static str,
    method_passkey: &'static str,
    // account created
    created_subject: &'static str,
    created_preheader: &'static str,
    created_text: &'static str,
    created_html: &'static str,
    created_button: &'static str,
    created_deadline_html: &'static str,
    // password changed
    password_subject: &'static str,
    password_preheader: &'static str,
    password_changed: &'static str,
    password_no_action: &'static str,
    password_warning_text: &'static str,
    password_warning_html: &'static str,
    // second factor added
    mfa_subject: &'static str,
    mfa_preheader: &'static str,
    mfa_added: &'static str,
    mfa_no_action: &'static str,
    mfa_warning_text: &'static str,
    mfa_warning_html: &'static str,
}

const FR: Words = Words {
    hello: "Bonjour",
    footer: "Cet email a été envoyé automatiquement par votre instance ArtiFerris.",
    method_authenticator_app: "une application d'authentification (TOTP)",
    method_passkey: "une clé d'accès (passkey)",
    created_subject: "Votre compte ArtiFerris",
    created_preheader: "Activez votre compte ArtiFerris",
    created_text: "Un compte ArtiFerris a été créé pour vous. Pour l'activer et choisir votre mot de passe, cliquez sur le lien suivant dans les 24 heures :",
    created_html: "Un compte ArtiFerris a été créé pour vous. Pour l'activer et choisir votre mot de passe, cliquez sur le bouton ci-dessous.",
    created_button: "Activer mon compte",
    created_deadline_html: "Ce lien expire dans 24 heures. Passé ce délai, demandez à un administrateur de vous renvoyer une invitation.",
    password_subject: "Votre mot de passe ArtiFerris a été modifié",
    password_preheader: "Votre mot de passe a été modifié",
    password_changed: "Le mot de passe de votre compte ArtiFerris vient d'être modifié.",
    password_no_action: "Si vous êtes à l'origine de ce changement, aucune action n'est nécessaire.",
    password_warning_text: "Si vous n'êtes pas à l'origine de ce changement, contactez immédiatement un administrateur de votre instance ArtiFerris.",
    password_warning_html: "Si vous n'êtes <strong>pas</strong> à l'origine de ce changement, contactez immédiatement un administrateur de votre instance ArtiFerris.",
    mfa_subject: "Nouvelle méthode de double authentification ajoutée",
    mfa_preheader: "Nouvelle méthode de double authentification",
    mfa_added: "Une nouvelle méthode de double authentification vient d'être ajoutée à votre compte ArtiFerris : {method}.",
    mfa_no_action: "Si vous êtes à l'origine de cet ajout, aucune action n'est nécessaire.",
    mfa_warning_text: "Si vous n'êtes pas à l'origine de cet ajout, contactez immédiatement un administrateur de votre instance ArtiFerris.",
    mfa_warning_html: "Si vous n'êtes <strong>pas</strong> à l'origine de cet ajout, contactez immédiatement un administrateur de votre instance ArtiFerris.",
};

const EN: Words = Words {
    hello: "Hello",
    footer: "This email was sent automatically by your ArtiFerris instance.",
    method_authenticator_app: "an authenticator app (TOTP)",
    method_passkey: "a passkey",
    created_subject: "Your ArtiFerris account",
    created_preheader: "Activate your ArtiFerris account",
    created_text: "An ArtiFerris account has been created for you. To activate it and choose your password, follow this link within 24 hours:",
    created_html: "An ArtiFerris account has been created for you. To activate it and choose your password, click the button below.",
    created_button: "Activate my account",
    created_deadline_html: "This link expires in 24 hours. After that, ask an administrator to send you a new invitation.",
    password_subject: "Your ArtiFerris password has been changed",
    password_preheader: "Your password has been changed",
    password_changed: "The password of your ArtiFerris account has just been changed.",
    password_no_action: "If you made this change, no action is needed.",
    password_warning_text: "If you did not make this change, contact an administrator of your ArtiFerris instance immediately.",
    password_warning_html: "If you did <strong>not</strong> make this change, contact an administrator of your ArtiFerris instance immediately.",
    mfa_subject: "New two-factor authentication method added",
    mfa_preheader: "New two-factor authentication method",
    mfa_added: "A new two-factor authentication method has just been added to your ArtiFerris account: {method}.",
    mfa_no_action: "If you added it, no action is needed.",
    mfa_warning_text: "If you did not add it, contact an administrator of your ArtiFerris instance immediately.",
    mfa_warning_html: "If you did <strong>not</strong> add it, contact an administrator of your ArtiFerris instance immediately.",
};

const ES: Words = Words {
    hello: "Hola",
    footer: "Este correo se ha enviado automáticamente desde su instancia de ArtiFerris.",
    method_authenticator_app: "una aplicación de autenticación (TOTP)",
    method_passkey: "una clave de acceso (passkey)",
    created_subject: "Su cuenta de ArtiFerris",
    created_preheader: "Active su cuenta de ArtiFerris",
    created_text: "Se ha creado una cuenta de ArtiFerris para usted. Para activarla y elegir su contraseña, siga este enlace en un plazo de 24 horas:",
    created_html: "Se ha creado una cuenta de ArtiFerris para usted. Para activarla y elegir su contraseña, haga clic en el botón de abajo.",
    created_button: "Activar mi cuenta",
    created_deadline_html: "Este enlace caduca en 24 horas. Pasado ese plazo, pida a un administrador que le envíe una nueva invitación.",
    password_subject: "Se ha cambiado su contraseña de ArtiFerris",
    password_preheader: "Se ha cambiado su contraseña",
    password_changed: "La contraseña de su cuenta de ArtiFerris se acaba de cambiar.",
    password_no_action: "Si usted hizo este cambio, no es necesaria ninguna acción.",
    password_warning_text: "Si usted no hizo este cambio, póngase en contacto de inmediato con un administrador de su instancia de ArtiFerris.",
    password_warning_html: "Si usted <strong>no</strong> hizo este cambio, póngase en contacto de inmediato con un administrador de su instancia de ArtiFerris.",
    mfa_subject: "Nuevo método de autenticación de dos factores añadido",
    mfa_preheader: "Nuevo método de autenticación de dos factores",
    mfa_added: "Se acaba de añadir un nuevo método de autenticación de dos factores a su cuenta de ArtiFerris: {method}.",
    mfa_no_action: "Si usted lo añadió, no es necesaria ninguna acción.",
    mfa_warning_text: "Si usted no lo añadió, póngase en contacto de inmediato con un administrador de su instancia de ArtiFerris.",
    mfa_warning_html: "Si usted <strong>no</strong> lo añadió, póngase en contacto de inmediato con un administrador de su instancia de ArtiFerris.",
};

const IT: Words = Words {
    hello: "Buongiorno",
    footer: "Questa e-mail è stata inviata automaticamente dalla tua istanza di ArtiFerris.",
    method_authenticator_app: "un'app di autenticazione (TOTP)",
    method_passkey: "una passkey",
    created_subject: "Il tuo account ArtiFerris",
    created_preheader: "Attiva il tuo account ArtiFerris",
    created_text: "È stato creato un account ArtiFerris per te. Per attivarlo e scegliere la password, segui questo link entro 24 ore:",
    created_html: "È stato creato un account ArtiFerris per te. Per attivarlo e scegliere la password, fai clic sul pulsante qui sotto.",
    created_button: "Attiva il mio account",
    created_deadline_html: "Questo link scade tra 24 ore. Trascorso questo termine, chiedi a un amministratore di inviarti un nuovo invito.",
    password_subject: "La tua password ArtiFerris è stata modificata",
    password_preheader: "La tua password è stata modificata",
    password_changed: "La password del tuo account ArtiFerris è stata appena modificata.",
    password_no_action: "Se sei stato tu a fare questa modifica, non è necessaria alcuna azione.",
    password_warning_text: "Se non sei stato tu a fare questa modifica, contatta subito un amministratore della tua istanza di ArtiFerris.",
    password_warning_html: "Se <strong>non</strong> sei stato tu a fare questa modifica, contatta subito un amministratore della tua istanza di ArtiFerris.",
    mfa_subject: "Nuovo metodo di autenticazione a due fattori aggiunto",
    mfa_preheader: "Nuovo metodo di autenticazione a due fattori",
    mfa_added: "Un nuovo metodo di autenticazione a due fattori è stato appena aggiunto al tuo account ArtiFerris: {method}.",
    mfa_no_action: "Se sei stato tu ad aggiungerlo, non è necessaria alcuna azione.",
    mfa_warning_text: "Se non sei stato tu ad aggiungerlo, contatta subito un amministratore della tua istanza di ArtiFerris.",
    mfa_warning_html: "Se <strong>non</strong> sei stato tu ad aggiungerlo, contatta subito un amministratore della tua istanza di ArtiFerris.",
};

const DE: Words = Words {
    hello: "Hallo",
    footer: "Diese E-Mail wurde automatisch von Ihrer ArtiFerris-Instanz gesendet.",
    method_authenticator_app: "eine Authentifizierungs-App (TOTP)",
    method_passkey: "ein Passkey",
    created_subject: "Ihr ArtiFerris-Konto",
    created_preheader: "Aktivieren Sie Ihr ArtiFerris-Konto",
    created_text: "Für Sie wurde ein ArtiFerris-Konto erstellt. Um es zu aktivieren und Ihr Passwort zu wählen, folgen Sie innerhalb von 24 Stunden diesem Link:",
    created_html: "Für Sie wurde ein ArtiFerris-Konto erstellt. Um es zu aktivieren und Ihr Passwort zu wählen, klicken Sie auf die Schaltfläche unten.",
    created_button: "Mein Konto aktivieren",
    created_deadline_html: "Dieser Link läuft in 24 Stunden ab. Danach bitten Sie einen Administrator, Ihnen eine neue Einladung zu senden.",
    password_subject: "Ihr ArtiFerris-Passwort wurde geändert",
    password_preheader: "Ihr Passwort wurde geändert",
    password_changed: "Das Passwort Ihres ArtiFerris-Kontos wurde gerade geändert.",
    password_no_action: "Wenn Sie diese Änderung vorgenommen haben, ist keine Aktion erforderlich.",
    password_warning_text: "Wenn Sie diese Änderung nicht vorgenommen haben, wenden Sie sich sofort an einen Administrator Ihrer ArtiFerris-Instanz.",
    password_warning_html: "Wenn Sie diese Änderung <strong>nicht</strong> vorgenommen haben, wenden Sie sich sofort an einen Administrator Ihrer ArtiFerris-Instanz.",
    mfa_subject: "Neue Zwei-Faktor-Methode hinzugefügt",
    mfa_preheader: "Neue Zwei-Faktor-Methode",
    mfa_added: "Ihrem ArtiFerris-Konto wurde gerade eine neue Zwei-Faktor-Methode hinzugefügt: {method}.",
    mfa_no_action: "Wenn Sie sie hinzugefügt haben, ist keine Aktion erforderlich.",
    mfa_warning_text: "Wenn Sie sie nicht hinzugefügt haben, wenden Sie sich sofort an einen Administrator Ihrer ArtiFerris-Instanz.",
    mfa_warning_html: "Wenn Sie sie <strong>nicht</strong> hinzugefügt haben, wenden Sie sich sofort an einen Administrator Ihrer ArtiFerris-Instanz.",
};

fn words(language: Language) -> &'static Words {
    match language {
        Language::En => &EN,
        Language::Fr => &FR,
        Language::Es => &ES,
        Language::It => &IT,
        Language::De => &DE,
    }
}

pub fn account_created(language: Language, activation_url: &str) -> EmailContent {
    let c = words(language);
    let hello = c.hello;
    let text = format!("{hello},\n\n{intro}\n\n{activation_url}\n\n{deadline}", intro = c.created_text, deadline = strip_tags(c.created_deadline_html));
    let body_html = format!(
        r#"<p style="margin:0 0 16px;">{hello},</p>
<p style="margin:0 0 16px;">{intro}</p>
{button}
<p style="margin:16px 0 0; font-size:13px; color:{TEXT_SECONDARY};">{deadline}</p>"#,
        intro = c.created_html,
        button = button(activation_url, c.created_button),
        deadline = c.created_deadline_html,
    );
    EmailContent { subject: c.created_subject.to_string(), text, html: shell(language, c.created_preheader, &body_html) }
}

pub fn password_changed(language: Language, username: &str) -> EmailContent {
    let c = words(language);
    let hello = c.hello;
    let text = format!("{hello} {username},\n\n{}\n\n{}\n\n{}", c.password_changed, c.password_no_action, c.password_warning_text);
    let body_html = format!(
        r#"<p style="margin:0 0 16px;">{hello} <strong>{username}</strong>,</p>
<p style="margin:0 0 16px;">{}</p>
<p style="margin:0 0 16px;">{}</p>
<p style="margin:0; padding:12px 16px; background-color:#fef3c7; border-radius:8px; color:#92400e;">{}</p>"#,
        c.password_changed, c.password_no_action, c.password_warning_html
    );
    EmailContent { subject: c.password_subject.to_string(), text, html: shell(language, c.password_preheader, &body_html) }
}

pub fn mfa_enrolled(language: Language, username: &str, method: EnrolledMethod) -> EmailContent {
    let c = words(language);
    let hello = c.hello;
    let method = match method {
        EnrolledMethod::AuthenticatorApp => c.method_authenticator_app,
        EnrolledMethod::Passkey => c.method_passkey,
    };
    let text = format!("{hello} {username},\n\n{}\n\n{}\n\n{}", c.mfa_added.replace("{method}", method), c.mfa_no_action, c.mfa_warning_text);
    let body_html = format!(
        r#"<p style="margin:0 0 16px;">{hello} <strong>{username}</strong>,</p>
<p style="margin:0 0 16px;">{}</p>
<p style="margin:0 0 16px;">{}</p>
<p style="margin:0; padding:12px 16px; background-color:#fef3c7; border-radius:8px; color:#92400e;">{}</p>"#,
        c.mfa_added.replace("{method}", &format!("<strong>{method}</strong>")),
        c.mfa_no_action,
        c.mfa_warning_html
    );
    EmailContent { subject: c.mfa_subject.to_string(), text, html: shell(language, c.mfa_preheader, &body_html) }
}

/// The plain-text version of a sentence that carries `<strong>` markup.
fn strip_tags(html: &str) -> String {
    html.replace("<strong>", "").replace("</strong>", "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use artiferris_domain::user_preferences::SUPPORTED_LANGUAGES;

    const URL: &str = "https://artiferris.example.com/activate?token=abc";

    #[test]
    fn account_created_includes_the_activation_link_in_both_bodies() {
        for language in SUPPORTED_LANGUAGES {
            let content = account_created(language, URL);
            assert!(content.text.contains(URL), "{language:?}");
            assert!(content.html.contains(URL), "{language:?}");
            assert!(content.html.starts_with("<!doctype html>"));
        }
    }

    #[test]
    fn every_email_is_written_in_the_requested_language() {
        let subjects = |language| {
            [
                account_created(language, URL).subject,
                password_changed(language, "florian").subject,
                mfa_enrolled(language, "florian", EnrolledMethod::Passkey).subject,
            ]
        };
        assert_eq!(subjects(Language::Fr)[0], "Votre compte ArtiFerris");
        assert_eq!(subjects(Language::En)[1], "Your ArtiFerris password has been changed");
        assert_eq!(subjects(Language::Es)[2], "Nuevo método de autenticación de dos factores añadido");
        assert_eq!(subjects(Language::It)[0], "Il tuo account ArtiFerris");
        assert_eq!(subjects(Language::De)[1], "Ihr ArtiFerris-Passwort wurde geändert");
    }

    #[test]
    fn the_html_declares_its_language_and_no_two_languages_share_a_subject() {
        let mut seen = std::collections::HashSet::new();
        for language in SUPPORTED_LANGUAGES {
            let content = password_changed(language, "florian");
            assert!(content.html.contains(&format!(r#"<html lang="{}">"#, language.as_str())));
            assert!(seen.insert(content.subject), "{language:?} repeats another language's subject");
        }
    }

    #[test]
    fn password_changed_mentions_the_username_in_both_bodies() {
        for language in SUPPORTED_LANGUAGES {
            let content = password_changed(language, "florian");
            assert!(content.text.contains("florian"));
            assert!(content.html.contains("florian"));
        }
        assert!(password_changed(Language::Fr, "florian").html.contains("modifié"));
    }

    #[test]
    fn mfa_enrolled_names_the_method_in_the_readers_language_in_both_bodies() {
        let fr = mfa_enrolled(Language::Fr, "florian", EnrolledMethod::Passkey);
        assert!(fr.text.contains("une clé d'accès (passkey)"));
        assert!(fr.html.contains("une clé d'accès (passkey)"));
        let de = mfa_enrolled(Language::De, "florian", EnrolledMethod::AuthenticatorApp);
        assert!(de.text.contains("eine Authentifizierungs-App (TOTP)"));
        assert!(de.html.contains("<strong>eine Authentifizierungs-App (TOTP)</strong>"));
    }

    #[test]
    fn no_placeholder_or_markup_leaks_into_the_plain_text_bodies() {
        for language in SUPPORTED_LANGUAGES {
            for content in [
                account_created(language, URL),
                password_changed(language, "florian"),
                mfa_enrolled(language, "florian", EnrolledMethod::AuthenticatorApp),
            ] {
                assert!(!content.text.contains('{') && !content.text.contains("<strong>"), "{language:?}: {}", content.text);
            }
        }
    }
}
