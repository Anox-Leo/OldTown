import tempfile
import unittest
from pathlib import Path

from otp_relay import config


class ConfigTests(unittest.TestCase):
    def load(self, idle_refresh: int) -> config.Config:
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "config.toml"
            path.write_text(f'[imap]\nusername = "moi@example.org"\nidle_refresh = {idle_refresh}\n')
            return config.load(path)

    def test_short_idle_refresh_is_accepted(self):
        self.assertEqual(self.load(10).imap.idle_refresh, 10)

    def test_idle_refresh_bounds(self):
        for bad in (4, 1501):
            with self.assertRaises(config.ConfigError):
                self.load(bad)


if __name__ == "__main__":
    unittest.main()
