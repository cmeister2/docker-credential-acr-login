export default {
  tagFormat: '${version}',
  plugins: [
    '@semantic-release/commit-analyzer',
    ['@semantic-release/release-notes-generator', {
      preset: 'conventionalcommits',
    }],
    '@semantic-release/github',
    ['@semantic-release/exec', {
      publishCmd: './publish.sh ${nextRelease.version}',
    }],
  ],
}