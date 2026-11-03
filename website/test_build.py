import json
from html.parser import HTMLParser
from pathlib import Path
import tempfile
import unittest
from urllib.parse import unquote, urlparse
from xml.etree import ElementTree

from build import ASSET_SUFFIXES, ROOT, SOURCE, build, release_links, write_discovery_files


class Links(HTMLParser):
    def __init__(self):
        super().__init__()
        self.local = []
        self.downloads = []
        self.current_languages = []
        self.alternates = {}
        self.metadata = {}
        self.og_alternates = set()
        self.canonical = None
        self.ids = set()
        self.schemas = []
        self.text = []
        self._json = None

    def handle_starttag(self, tag, attributes):
        attrs = dict(attributes)
        if "id" in attrs:
            self.ids.add(attrs["id"])
        if tag == "script" and attrs.get("type") == "application/ld+json":
            self._json = ""
        if tag == "meta":
            key = attrs.get("name", attrs.get("property"))
            self.metadata[key] = attrs.get("content")
            if key == "og:locale:alternate":
                self.og_alternates.add(attrs["content"])
        if tag == "link" and attrs.get("rel") == "canonical":
            self.canonical = attrs["href"]
        if tag == "a" and attrs.get("aria-current") == "page":
            self.current_languages.append(attrs.get("lang"))
        if tag == "link" and attrs.get("rel") == "alternate":
            self.alternates[attrs["hreflang"]] = attrs["href"]
        for key, value in attributes:
            if key not in ("src", "href") or not value:
                continue
            if "/releases/download/" in value:
                self.downloads.append(value)
            if value.startswith(("./", "../")):
                self.local.append(value)

    def handle_data(self, data):
        if self._json is not None:
            self._json += data
        else:
            self.text.append(data)

    def handle_endtag(self, tag):
        if tag == "script" and self._json is not None:
            self.schemas.append(json.loads(self._json))
            self._json = None


class HomepageTests(unittest.TestCase):
    def setUp(self):
        self.config = json.loads((SOURCE / "site.json").read_text())
        self.repository = self.config["repository"]
        self.release = {
            "tag_name": "v9.8.7", "draft": False, "prerelease": False,
            "html_url": self.repository + "/releases/tag/v9.8.7",
            "assets": [{
                "name": "patchlook-9.8.7" + suffix,
                "browser_download_url": self.repository + "/releases/download/v9.8.7/patchlook-9.8.7" + suffix,
            } for suffix in ASSET_SUFFIXES.values()],
        }

    def test_localized_pages_have_working_relative_links_and_current_downloads(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "patchlook"
            build(self.release, output, "https://example.org/patchlook")
            for locale, path in self.config["languages"].items():
                page = output / path / "index.html"
                markup = page.read_text()
                self.assertIn(f'<html lang="{locale}">', markup)
                self.assertIn("v9.8.7", markup)
                parsed = Links()
                parsed.feed(markup)
                self.assertEqual(len(parsed.downloads), 3)
                self.assertEqual(parsed.current_languages, [locale])
                self.assertEqual(parsed.alternates, {
                    **{code: f"https://example.org/patchlook/{route + '/' if route else ''}"
                       for code, route in self.config["languages"].items()},
                    "x-default": "https://example.org/patchlook/",
                })
                readme = "README.zh-CN.md#安装" if locale.startswith("zh-") else "README.md#install"
                self.assertIn(f"{self.repository}/blob/main/{readme}", markup)
                self.assertIn(f'brew install --cask {self.config["homebrew_cask"]}', markup)
                self.assertNotIn('brew upgrade', markup)
                for value in parsed.local:
                    target = page.parent / unquote(urlparse(value).path)
                    self.assertTrue(target.exists(), str(target))
                for url in parsed.downloads:
                    self.assertIn("/v9.8.7/", url)
            sitemap = ElementTree.parse(output / "sitemap.xml")
            urls = {element.text for element in sitemap.findall("{*}url/{*}loc")}
            self.assertEqual(urls, {
                f"https://example.org/patchlook/{path + '/' if path else ''}"
                for path in self.config["languages"].values()
            })
            for entry in sitemap.findall("{*}url"):
                self.assertEqual({link.get("hreflang"): link.get("href") for link in entry.findall("{*}link")}, parsed.alternates)

    def test_metadata_describes_visible_content_and_the_published_release(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            build(self.release, output)
            translations = {locale: json.loads((SOURCE / f"locales/{locale}.json").read_text())
                            for locale in self.config["languages"]}
            for locale, route in self.config["languages"].items():
                with self.subTest(locale=locale):
                    copy = translations[locale]
                    parsed = Links()
                    parsed.feed((output / route / "index.html").read_text())
                    canonical = f'{self.config["url"]}/{route + "/" if route else ""}'
                    self.assertEqual(parsed.canonical, canonical)
                    self.assertEqual(parsed.metadata["og:url"], canonical)
                    self.assertEqual(parsed.metadata["og:locale"], copy["og_locale"])
                    self.assertEqual(parsed.og_alternates, {c["og_locale"] for code, c in translations.items() if code != locale})
                    self.assertEqual(parsed.metadata["twitter:title"], copy["title"])
                    self.assertEqual(parsed.metadata["twitter:description"], copy["description"])
                    self.assertEqual(parsed.metadata["og:image"], parsed.metadata["twitter:image"])
                    self.assertEqual(parsed.metadata["og:image:alt"], copy["screenshot_alt"])
                    self.assertEqual(len(parsed.schemas), 1)
                    page, app = parsed.schemas[0]["@graph"]
                    self.assertEqual(page["@id"], canonical)
                    self.assertEqual(page["inLanguage"], locale)
                    self.assertEqual(page["mainEntity"]["@id"], app["@id"])
                    self.assertEqual(app["@id"], self.config["url"] + "/#app")
                    self.assertEqual(app["softwareVersion"], "9.8.7")
                    self.assertEqual(set(app["downloadUrl"]), set(parsed.downloads))
                    self.assertEqual(app["offers"]["price"], 0)
                    self.assertNotIn("aggregateRating", app)
                    self.assertNotIn("review", app)
                    self.assertEqual(app["license"], self.repository + "/blob/main/LICENSE")
                    visible_text = "".join(parsed.text)
                    for fact in [app["description"], *app["featureList"], *app["operatingSystem"]]:
                        self.assertIn(fact, visible_text)
                    for topic in ("platforms", "staging", "ssh"):
                        self.assertIn(copy[f"faq_{topic}_answer"].format_map(copy), visible_text)
                    self.assertTrue({"platform-support", "local-staging", "remote-ssh"} <= parsed.ids)

    def test_json_ld_escapes_script_delimiters_without_changing_values(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            version = 'v9.8.7</script><script> & "quoted"'
            build({**self.release, "tag_name": version}, output)
            markup = (output / "index.html").read_text()
            self.assertNotIn(version, markup)
            parsed = Links()
            parsed.feed(markup)
            self.assertEqual(len(parsed.schemas), 1)
            self.assertEqual(parsed.schemas[0]["@graph"][1]["softwareVersion"], version[1:])

    def test_robots_rules_are_only_generated_for_domain_root_deployments(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            write_discovery_files(output, "https://example.org", self.config["languages"])
            self.assertIn("Sitemap: https://example.org/sitemap.xml", (output / "robots.txt").read_text())
            write_discovery_files(output, "https://example.org/patchlook", self.config["languages"])
            self.assertFalse((output / "robots.txt").exists())
            self.assertTrue((output / "sitemap.xml").exists())

    def test_rejects_invalid_canonical_site_urls(self):
        with tempfile.TemporaryDirectory() as directory:
            for url in ("/patchlook", "https:///patchlook", "ftp://example.org", "https://example.org/?q=1", "https://example.org/#part"):
                with self.subTest(url=url), self.assertRaises(ValueError):
                    build(self.release, Path(directory), url)

    def test_locales_match_application_and_have_complete_copy(self):
        languages = self.config["languages"]
        self.assertEqual(set(languages), {path.stem for path in (ROOT / "locales").glob("*.json")})
        self.assertEqual(set(languages), {path.stem for path in (SOURCE / "locales").glob("*.json")})
        self.assertEqual(languages["en"], "")
        self.assertEqual(len(set(languages.values())), len(languages))
        english = json.loads((SOURCE / "locales/en.json").read_text())
        for locale in languages:
            copy = json.loads((SOURCE / f"locales/{locale}.json").read_text())
            with self.subTest(locale=locale):
                self.assertEqual(copy.keys(), english.keys())
                self.assertEqual(copy["lang"], locale)
                self.assertTrue(all(isinstance(value, str) and value.strip() for value in copy.values()))

    def test_rejects_unpublished_and_incomplete_releases(self):
        for field in ("draft", "prerelease"):
            with self.subTest(field=field), self.assertRaises(ValueError):
                release_links({**self.release, field: True}, self.repository)
        with self.assertRaises(ValueError):
            release_links({**self.release, "assets": self.release["assets"][:2]}, self.repository)
        with self.assertRaises(ValueError):
            release_links({**self.release, "html_url": "https://example.org/release"}, self.repository)


if __name__ == "__main__":
    unittest.main()
