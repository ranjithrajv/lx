use lx_lib::bitbucket::*;

#[test]
fn maps_downloads_to_release() {
    let raw = BitbucketDownloadsRaw {
        values: vec![BitbucketDownloadRaw {
            name: "tool.tar.gz".into(),
            size: Some(123),
            links: Some(BitbucketLinksRaw {
                download: Some(BitbucketHref {
                    href: Some("https://bitbucket.org/a/b/downloads/tool.tar.gz".into()),
                }),
                self_: None,
            }),
        }],
        pagelen: Some(10),
        page: Some(1),
    };
    let r = BitbucketClient::map_downloads_to_release(raw, "a/b");
    assert_eq!(r.tag_name, "latest");
    assert_eq!(r.assets[0].name, "tool.tar.gz");
}
