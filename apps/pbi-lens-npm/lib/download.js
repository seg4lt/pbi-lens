'use strict';

const fs = require('node:fs');
const { Readable } = require('node:stream');
const { pipeline } = require('node:stream/promises');

async function download(url, destination) {
  const response = await fetch(url, { headers: { 'User-Agent': '@seg4lt/pbi-lens-installer' }, redirect: 'follow' });
  if (!response.ok || !response.body) throw new Error(`Download failed with HTTP ${response.status}`);
  await pipeline(Readable.fromWeb(response.body), fs.createWriteStream(destination));
}

module.exports = { download };
