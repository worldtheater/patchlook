"""Build the dependency-free Pages site from a published GitHub release."""

import argparse
import html
import json
from pathlib import Path
import re
import shutil
from string import Template
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "website"
ASSET_SUFFIXES = {
    "mac": "-darwin-arm64.dmg",
    "windows": "-windows-amd64.zip",
    "linux": "-linux-x86_64.tar.gz",
}


def svg(paths, css_class=""):
    return (
        f'<svg class="{css_class}" viewBox="0 0 24 24" fill="none" '
        'stroke="currentColor" stroke-width="1.7" stroke-linecap="round" '
        f'stroke-linejoin="round" aria-hidden="true">{paths}</svg>'
    )


ICONS = {
    "close": '<path d="m6 6 12 12M6 18 18 6"/>',
    "copy": '<rect x="8" y="8" width="12" height="13" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/>',
    "download": '<path d="M12 3v12m-5-5 5 5 5-5M5 16v4h14v-4"/>',
    "lock": '<rect x="6" y="10" width="12" height="11" rx="2"/><path d="M8 10V6a4 4 0 0 1 8 0v4m-4 5v2"/>',
    "check": '<rect x="3" y="3" width="18" height="18" rx="4"/><path d="m7 12 3 3 7-7"/>',
    "terminal": '<rect x="2" y="4" width="20" height="16" rx="3"/><path d="m6 9 3 3-3 3m7 0h4"/>',
    "mac": '<rect x="3" y="3" width="18" height="13" rx="2"/><path d="M8 21h8m-4-5v5"/>',
    "windows": '<path d="M3 5 21 2v20L3 19zm8-1.3v16.6M3 12h18"/>',
    "external": '<path d="M14 3h7v7m0-7L10 14M10 3H4a1 1 0 0 0-1 1v16a1 1 0 0 0 1 1h16a1 1 0 0 0 1-1v-6"/>',
    "expand": '<path d="M8 3H3v5m13-5h5v5M3 16v5h5m13-5v5h-5"/>',
    "chevron": '<path d="m9 5 7 7-7 7"/>',
    "sun": '<circle cx="12" cy="12" r="4"/><path d="M12 2v1m0 18v1M2 12h1m18 0h1M5 5l1 1m12 12 1 1M5 19l1-1M18 6l1-1"/>',
    "moon": '<path d="M20.5 14A9 9 0 0 1 10 3.5 9 9 0 1 0 20.5 14Z"/>',
    "language": '<path d="M2 5h12M8 3v2m4 0c-1 6-4 9-9 11m2-8c1 3 4 6 7 7m1 6 5-12 5 12m-8-4h6"/>',
    "tick": '<path d="m5 12 4 4L19 6"/>',
    "github": '<path fill="currentColor" stroke="none" d="M12 0C5.37 0 0 5.37 0 12c0 5.3 3.44 9.8 8.21 11.39.6.11.82-.26.82-.58v-2.23c-3.34.73-4.04-1.42-4.04-1.42-.55-1.39-1.33-1.76-1.33-1.76-1.09-.75.08-.73.08-.73 1.21.09 1.85 1.24 1.85 1.24 1.07 1.84 2.81 1.31 3.49 1 .11-.78.42-1.31.76-1.61-2.67-.3-5.47-1.34-5.47-5.93 0-1.31.47-2.38 1.24-3.22-.13-.3-.54-1.52.12-3.18 0 0 1.01-.32 3.3 1.23A11.5 11.5 0 0 1 12 5.8c1.02 0 2.05.14 3.01.4 2.29-1.55 3.3-1.23 3.3-1.23.66 1.66.25 2.88.12 3.18.77.84 1.24 1.91 1.24 3.22 0 4.6-2.8 5.62-5.48 5.92.43.37.82 1.1.82 2.22v3.3c0 .32.22.69.83.57A12 12 0 0 0 24 12C24 5.37 18.63 0 12 0Z"/>',
}


def release_links(release, repository):
    """Require actual published assets; do not guess versioned download URLs."""
    if release.get("draft") is not False or release.get("prerelease") is not False:
        raise ValueError("The homepage requires a published stable release")
    release_url = release["html_url"]
    if not release_url.startswith(repository + "/releases/tag/"):
        raise ValueError("Release does not belong to the configured repository")
    result = {"version": release["tag_name"], "release_url": release_url}
    for platform, suffix in ASSET_SUFFIXES.items():
        assets = [asset for asset in release["assets"] if asset["name"].endswith(suffix)]
        if len(assets) != 1:
            raise ValueError(f"Expected one {platform} release asset, got {len(assets)}")
        asset = assets[0]
        url = asset["browser_download_url"]
        if not url.startswith(repository + "/releases/download/"):
            raise ValueError(f"Unexpected download URL for {platform}")
        result[f"{platform}_url"] = url
        digest = asset.get("digest")
        if not isinstance(digest, str) or not re.fullmatch(r"sha256:[0-9a-fA-F]{64}", digest):
            raise ValueError(f'Missing or invalid SHA-256 for {asset["name"]}')
        result[f"{platform}_sha256"] = digest.removeprefix("sha256:").lower()
    return result


def structured_data(config, page):
    """Describe the app using the same localized facts and release as the page."""
    app_id = f'{page["site_url"]}/#app'
    data = {
        "@context": "https://schema.org",
        "@graph": [
            {
                "@type": "WebPage", "@id": page["canonical"],
                "url": page["canonical"], "name": page["title"],
                "description": page["description"], "inLanguage": page["lang"],
                "mainEntity": {"@id": app_id},
            },
            {
                "@type": "SoftwareApplication", "@id": app_id,
                "name": config["name"], "url": f'{page["site_url"]}/',
                "description": page["intro"],
                "applicationCategory": "DeveloperApplication",
                "operatingSystem": [page[f"{platform}_requirements"] for platform in ASSET_SUFFIXES],
                "inLanguage": list(config["languages"]),
                "softwareVersion": page["version"].removeprefix("v"),
                "releaseNotes": page["release_url"],
                "downloadUrl": [page[f"{platform}_url"] for platform in ASSET_SUFFIXES],
                "installUrl": page["install_url"],
                "license": f'{page["repository"]}/blob/main/LICENSE',
                "sameAs": page["repository"],
                "image": f'{page["site_url"]}/assets/logo.png',
                "screenshot": f'{page["site_url"]}/assets/patchlook-light-dark.webp',
                "isAccessibleForFree": True,
                "offers": {"@type": "Offer", "price": 0, "url": page["release_url"]},
                "featureList": [page[f"{feature}_body"] for feature in ("review", "stage", "ssh")],
            },
        ],
    }
    # JSON-LD is script text, not HTML: escape tag delimiters without HTML entities.
    return json.dumps(data, ensure_ascii=False).replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")


def write_discovery_files(output, site_url, languages):
    urls = {locale: f"{site_url}/{path + '/' if path else ''}" for locale, path in languages.items()}
    alternates = {**urls, "x-default": f"{site_url}/"}
    language_links = "".join(
        f'    <xhtml:link rel="alternate" hreflang="{html.escape(locale)}" href="{html.escape(url)}"/>\n'
        for locale, url in alternates.items()
    )
    (output / "sitemap.xml").write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">\n'
        + "".join(f"  <url>\n    <loc>{html.escape(url)}</loc>\n{language_links}  </url>\n" for url in urls.values())
        + "</urlset>\n", encoding="utf-8"
    )
    robots = output / "robots.txt"
    if not urlparse(site_url).path:
        robots.write_text(f"User-agent: *\nAllow: /\nSitemap: {site_url}/sitemap.xml\n", encoding="utf-8")
    else:
        # A project Pages subdirectory cannot control the host's /robots.txt.
        robots.unlink(missing_ok=True)


def build(release, output, site_url=None):
    config = json.loads((SOURCE / "site.json").read_text())
    repository = config["repository"]
    site_url = (site_url or config["url"]).rstrip("/")
    parsed_url = urlparse(site_url)
    if parsed_url.scheme not in ("http", "https") or not parsed_url.netloc or parsed_url.query or parsed_url.fragment:
        raise ValueError("Site URL must be an absolute HTTP(S) URL without a query or fragment")
    links = release_links(release, repository)
    template = Template((SOURCE / "index.html").read_text())
    english = json.loads((SOURCE / "locales/en.json").read_text())
    languages = config["languages"]
    translations = {locale: json.loads((SOURCE / f"locales/{locale}.json").read_text()) for locale in languages}
    language_names = {
        locale: json.loads((ROOT / f"locales/{locale}.json").read_text())["language_name"]
        for locale in languages
    }
    alternate_links = "\n  ".join(
        f'<link rel="alternate" hreflang="{html.escape(locale)}" '
        f'href="{html.escape(site_url)}/{directory + "/" if directory else ""}">'
        for locale, directory in languages.items()
    )
    for locale, directory in languages.items():
        copy = translations[locale].copy()
        if copy.keys() != english.keys():
            raise ValueError(f"Translation keys do not match: {locale}")
        copy["faq_platforms_answer"] = copy["faq_platforms_answer"].format_map(copy)
        prefix = "../" if directory else "./"
        readme = "README.zh-CN.md" if locale in ("zh-CN", "zh-TW") else "README.md"
        values = {
            **copy, **links,
            "repository": repository,
            "site_url": site_url,
            "canonical": f"{site_url}/{directory + '/' if directory else ''}",
            "prefix": prefix,
            "home": "./",
            "guide_url": f"{repository}/blob/main/{readme}",
            "install_url": f"{repository}/blob/main/{readme}#{'安装' if readme == 'README.zh-CN.md' else 'install'}",
            "brew_install_command": f"brew install --cask {config['homebrew_cask']}",
            "homebrew_url": config["homebrew_url"],
        }
        schema = structured_data(config, values)
        values = {key: html.escape(str(value), quote=True) for key, value in values.items()}
        values["structured_data"] = schema
        values["og_alternate_locales"] = "\n  ".join(
            f'<meta property="og:locale:alternate" content="{html.escape(other["og_locale"], quote=True)}">'
            for code, other in translations.items() if code != locale
        )
        values.update({f"{key}_icon": svg(paths, key) for key, paths in ICONS.items()})
        values["alternate_links"] = alternate_links
        values["language_options"] = "\n          ".join(
            f'<a href="{prefix}{path + "/" if path else ""}" lang="{code}" '
            f'hreflang="{code}" aria-current="{"page" if code == locale else "false"}">'
            f'<span>{html.escape(language_names[code])}</span>{svg(ICONS["tick"], "tick")}</a>'
            for code, path in languages.items()
        )
        destination = output / directory
        destination.mkdir(parents=True, exist_ok=True)
        (destination / "index.html").write_text(template.substitute(values), encoding="utf-8")
    assets = output / "assets"
    assets.mkdir(exist_ok=True)
    shutil.copyfile(ROOT / "assets/app-icon/patchlook-v2.png", assets / "logo.png")
    shutil.copyfile(ROOT / "docs/images/patchlook-light-dark.webp", assets / "patchlook-light-dark.webp")
    for filename in ("styles.css", "theme.js"):
        shutil.copyfile(SOURCE / filename, output / filename)
    (output / ".nojekyll").touch()
    write_discovery_files(output, site_url, languages)
    print(f"Built {links['version']} homepage: {output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-json", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=ROOT / "dist/site")
    parser.add_argument("--site-url", help="Override the canonical URL (for local preview)")
    args = parser.parse_args()
    build(json.loads(args.release_json.read_text()), args.output, args.site_url)
