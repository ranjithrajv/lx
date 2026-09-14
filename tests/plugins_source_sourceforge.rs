// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::plugins::forge::sourceforge::parse_sourceforge_url;
use lx_lib::sourceforge::parse_rss_assets;

#[test]
fn parses_sourceforge_project_urls() {
    assert_eq!(
        parse_sourceforge_url("https://sourceforge.net/projects/sevenzip/").as_deref(),
        Some("sevenzip")
    );
    assert_eq!(
        parse_sourceforge_url("https://sourceforge.net/projects/sevenzip/files/7-Zip/26.03/")
            .as_deref(),
        Some("sevenzip")
    );
    assert_eq!(
        parse_sourceforge_url("https://sevenzip.sourceforge.net/").as_deref(),
        Some("sevenzip")
    );
}

#[test]
fn rejects_non_sourceforge() {
    assert!(parse_sourceforge_url("https://github.com/owner/repo").is_none());
    assert!(parse_sourceforge_url("package.yaml").is_none());
    assert!(parse_sourceforge_url("").is_none());
}

const FEED: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:media="http://video.search.yahoo.com/mrss/">
  <channel>
    <item>
      <title><![CDATA[/7-Zip/26.03/7z2603-linux-x64.tar.xz]]></title>
      <link>https://sourceforge.net/projects/sevenzip/files/7-Zip/26.03/7z2603-linux-x64.tar.xz/download</link>
      <media:content url="https://sourceforge.net/projects/sevenzip/files/7-Zip/26.03/7z2603-linux-x64.tar.xz/download" filesize="1572504"><media:hash algo="md5">e8ad00bc2b732b7f030949ffecd1dcc0</media:hash></media:content>
    </item>
    <item>
      <title><![CDATA[/7-Zip/26.03/7z2603-x64.exe]]></title>
      <link>https://sourceforge.net/projects/sevenzip/files/7-Zip/26.03/7z2603-x64.exe/download</link>
      <media:content url="x" filesize="1661239"><media:hash algo="md5">ece29f11bca82d84b4f06a0b73f127dc</media:hash></media:content>
    </item>
  </channel>
</rss>"#;

#[test]
fn parses_rss_items_into_sorted_assets() {
    let assets = parse_rss_assets(FEED).unwrap();
    assert_eq!(assets.len(), 2);
    // Sorted by asset name: after the shared `7z2603-` prefix, 'l' < 'x',
    // so the linux tarball sorts before the Windows installer.
    assert_eq!(assets[0].name, "7z2603-linux-x64.tar.xz");
    assert_eq!(assets[1].name, "7z2603-x64.exe");
    assert_eq!(assets[0].size, Some(1572504));
    assert!(assets[0]
        .browser_download_url
        .ends_with("/7z2603-linux-x64.tar.xz/download"));
    // The feed's inline md5 is carried on the asset for verification.
    assert_eq!(
        assets[0].checksums.get("md5").map(String::as_str),
        Some("e8ad00bc2b732b7f030949ffecd1dcc0")
    );
}

#[test]
fn empty_feed_yields_no_assets() {
    let assets = parse_rss_assets("<rss><channel></channel></rss>").unwrap();
    assert!(assets.is_empty());
}
