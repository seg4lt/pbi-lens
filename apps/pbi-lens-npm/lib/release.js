'use strict';

const REPO = 'seg4lt/pbi-lens';

async function latestRelease() {
  const response = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
    headers: { Accept: 'application/vnd.github+json', 'User-Agent': '@seg4lt/pbi-lens-installer' },
  });
  if (!response.ok) throw new Error(`GitHub returned HTTP ${response.status} while resolving the latest release`);
  const release = await response.json();
  const asset = release.assets?.find((item) => /^PBI-Lens-v.+-macos-arm64\.dmg$/.test(item.name));
  if (!asset?.browser_download_url) throw new Error('The latest release has no macOS Apple Silicon DMG');
  return { tag: release.tag_name, name: asset.name, url: asset.browser_download_url };
}

module.exports = { latestRelease };
