import unittest
from email.message import EmailMessage

from otp_relay.extract import detect, parse


def mail(subject: str, *, plain: str | None = None, html: str | None = None,
         sender: str = "Service <no-reply@service.example>", auth: str | None = None) -> bytes:
    msg = EmailMessage()
    msg["From"] = sender
    msg["To"] = "moi@gmail.com"
    msg["Subject"] = subject
    msg["Message-ID"] = "<abc@service.example>"
    if auth:
        msg["Authentication-Results"] = auth
    if plain is not None:
        msg.set_content(plain)
    if html is not None:
        if plain is None:
            msg.set_content(html, subtype="html")
        else:
            msg.add_alternative(html, subtype="html")
    return bytes(msg)


def code_of(raw: bytes) -> str | None:
    found = detect(parse(raw))
    return found.code if found else None


class DetectTests(unittest.TestCase):
    def test_plain_french(self):
        raw = mail("Votre code de connexion", plain="Bonjour,\n\nVotre code de vérification est : 482913\n\nIl expire dans 10 minutes.")
        self.assertEqual(code_of(raw), "482913")

    def test_code_after_number(self):
        raw = mail("Sign in", plain="482913 is your verification code. Do not share it.")
        self.assertEqual(code_of(raw), "482913")

    def test_google_prefix(self):
        raw = mail("G-583920 est votre code de validation Google", plain="Votre code de validation Google : G-583920")
        found = detect(parse(raw))
        self.assertEqual(found.code, "583920")
        self.assertEqual(found.display, "G-583920")

    def test_html_emphasized_isolated(self):
        html = """<html><head><style>.x{color:#123456}</style><title>Code 999999</title></head><body>
        <table><tr><td>Hi Alex, use the code below to sign in to your account.</td></tr>
        <tr><td style="font-size:32px;letter-spacing:6px"><b>731 044</b></td></tr>
        <tr><td>Order history · 2026 · <a href="https://x.example/u/12345678">unsubscribe</a></td></tr></table></body></html>"""
        found = detect(parse(mail("Your sign-in code", html=html)))
        self.assertEqual(found.code, "731044")
        self.assertGreaterEqual(found.confidence, 0.8)

    def test_alphanumeric(self):
        html = "<p>Your one-time passcode:</p><div style='font-family:monospace;font-size:24px'>X7K9QP</div>"
        self.assertEqual(code_of(mail("Verify your email", html=html)), "X7K9QP")

    def test_newsletter_without_code(self):
        raw = mail("Votre commande 48291355 est expédiée",
                   plain="Commande n° 48291355 du 12 octobre 2026.\nTotal : 1299 €\nSuivi colis : 88392011\nService client : 0145678912")
        self.assertIsNone(code_of(raw))

    def test_promo_code_ignored(self):
        raw = mail("-20% ce week-end", plain="Utilisez le code promo SUMMER24 avant le 31 août 2026. Offre valable sur 1500 articles.")
        self.assertIsNone(code_of(raw))

    def test_years_and_amounts_not_chosen(self):
        raw = mail("Security code", plain="© 2026 Example Inc.\nYour security code is 5521.\nYou paid 4500 € last month.")
        self.assertEqual(code_of(raw), "5521")

    def test_no_keyword_no_detection(self):
        self.assertIsNone(code_of(mail("Réunion", plain="On se retrouve salle 4012 à 14h.")))

    def test_invisible_chars(self):
        raw = mail("Code", plain="Votre code​ : 1​2​3​4​5​6")
        self.assertEqual(code_of(raw), "123456")

    def test_url_digits_ignored(self):
        raw = mail("Login", plain="Your login code is 662210.\nhttps://example.com/verify?token=99887766&id=123456")
        self.assertEqual(code_of(raw), "662210")

    def test_authentication_results_trusted_only_from_gmail(self):
        raw = mail("Code", plain="Code : 123456",
                   auth="mx.google.com; dkim=pass header.i=@service.example; spf=pass; dmarc=pass (p=REJECT) header.from=service.example")
        p = parse(raw)
        self.assertEqual(p.auth, {"dkim": "pass", "spf": "pass", "dmarc": "pass"})
        forged = mail("Code", plain="Code : 123456", auth="evil.example; dmarc=pass")
        self.assertEqual(parse(forged).auth, {})

    def test_metadata(self):
        p = parse(mail("Code", plain="Code : 123456", sender="GitHub <noreply@GitHub.com>"))
        self.assertEqual(p.from_domain, "github.com")
        self.assertEqual(p.to_address, "moi@gmail.com")
        self.assertEqual(len(p.event_id), 16)


if __name__ == "__main__":
    unittest.main()
