#!/usr/bin/env node
'use strict';

const fs = require('fs');
const path = require('path');

const repo = process.env.GITHUB_REPOSITORY;
const tag = process.env.RELEASE_TAG;

if (!repo || !tag) {
  console.error('GITHUB_REPOSITORY and RELEASE_TAG env vars are required');
  process.exit(1);
}

const version = tag.replace(/^v/, '');
const baseUrl = `https://github.com/${repo}/releases/download/${tag}`;

function readSig(dir, suffix) {
  const files = fs.readdirSync(dir).filter(f => f.endsWith(suffix));
  if (!files.length) throw new Error(`No file matching *${suffix} in ${dir}`);
  return fs.readFileSync(path.join(dir, files[0]), 'utf8').trim();
}

function extractChangelog(ver) {
  try {
    const text = fs.readFileSync('CHANGELOG.md', 'utf8');
    const re = new RegExp(`## \\[${ver}\\][^\\n]*\\n([\\s\\S]*?)(?=\\n## |$)`);
    const m = text.match(re);
    return m ? m[1].trim() : '';
  } catch {
    return '';
  }
}

const macosSig = readSig('sigs/macos', '.app.tar.gz.sig');
const windowsSig = readSig('sigs/windows', '.exe.sig');
const notes = extractChangelog(version);

const macosEntry = {
  signature: macosSig,
  url: `${baseUrl}/mtool_${tag}.app.tar.gz`,
};

const payload = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    'darwin-aarch64': macosEntry,
    'windows-x86_64': {
      signature: windowsSig,
      url: `${baseUrl}/mtool_${tag}.exe`,
    },
  },
};

fs.writeFileSync('latest.json', JSON.stringify(payload, null, 2));
console.log('Generated latest.json:\n', JSON.stringify(payload, null, 2));
