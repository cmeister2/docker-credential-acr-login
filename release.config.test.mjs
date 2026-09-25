import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { test } from 'node:test'
import { pathToFileURL } from 'node:url'
import releaseConfig from './release.config.js'

test('release notes render the Conventional Commits JavaScript templates', async () => {
  const localRequire = createRequire(import.meta.url)
  const releaseRequire = createRequire(localRequire.resolve('semantic-release'))
  const [, pluginConfig] = releaseConfig.plugins.find(plugin =>
    Array.isArray(plugin) && plugin[0] === '@semantic-release/release-notes-generator'
  )
  const { generateNotes } = await import(pathToFileURL(releaseRequire.resolve('@semantic-release/release-notes-generator')).href)
  const repositoryUrl = 'https://github.com/cmeister2/docker-credential-acr-login'
  const notes = await generateNotes(pluginConfig, {
    cwd: import.meta.dirname,
    options: { repositoryUrl },
    lastRelease: { gitTag: '1.0.0' },
    nextRelease: { version: '2.0.0', gitTag: '2.0.0' },
    commits: [
      {
        hash: '1234567890123456789012345678901234567890',
        message: 'feat: add credential controls',
      },
      {
        hash: 'abcdef0123456789abcdef0123456789abcdef01',
        message: 'fix: repair release notes\n\nCloses #42',
      },
      {
        hash: '0123456789abcdef0123456789abcdef01234567',
        message: 'feat!: require a mode\n\nBREAKING CHANGE: Callers must pass an explicit mode.',
      },
    ],
  })

  for (const expected of [
    `[2.0.0](${repositoryUrl}/compare/1.0.0...2.0.0)`,
    '### Features',
    '### Bug Fixes',
    'BREAKING CHANGES',
    'Callers must pass an explicit mode.',
    'add credential controls',
    'repair release notes',
    `${repositoryUrl}/issues/42`,
    `${repositoryUrl}/commit/1234567890123456789012345678901234567890`,
  ]) {
    assert.ok(notes.includes(expected), `Missing ${JSON.stringify(expected)} in release notes:\n${notes}`)
  }
})