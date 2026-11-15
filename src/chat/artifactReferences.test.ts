import { describe, expect, it } from 'vitest'
import { artifactReferenceId, referencedArtifactIds } from './artifactReferences'

describe('artifact references', () => {
  it('accepts only stable artifact identifiers', () => {
    expect(artifactReferenceId('artifact:art_123')).toBe('art_123')
    expect(artifactReferenceId('artifact://art_123')).toBe('art_123')
    for (const url of ['artifact:../../a', 'https://example.com/art_a', 'artifact:report.png', 'artifact:art_a/other']) {
      expect(artifactReferenceId(url)).toBeNull()
    }
  })

  it('collects real inline and reference links, excluding code examples and unused definitions', () => {
    const ids = referencedArtifactIds('[Report](artifact:art_report)\n\n![Chart](artifact:art_chart)\n\n[Video][clip]\n\n[clip]: artifact:art_video\n[unused]: artifact:art_unused\n\n`[example](artifact:art_inline)`\n\n```md\n![example](artifact:art_code)\n```')
    expect([...ids]).toEqual(['art_report', 'art_chart', 'art_video'])
  })
})

describe('artifact references across message texts', () => {
  it('reads each mentioning text on its own, so unrelated process text cannot hide a delivery', () => {
    const ids = referencedArtifactIds(['```sh\nnpm run build', 'plain reasoning', '[Report](artifact:art_report)'])
    expect([...ids]).toEqual(['art_report'])
  })

  it('still resolves a definition written in a different text than its link', () => {
    const ids = referencedArtifactIds(['[Video][clip]', '[clip]: artifact:art_video'])
    expect([...ids]).toEqual(['art_video'])
  })

  it('returns the same ids when a text is asked again', () => {
    const texts = ['![Chart](artifact:art_chart)', 'no reference here']
    expect([...referencedArtifactIds(texts)]).toEqual(['art_chart'])
    expect([...referencedArtifactIds(texts)]).toEqual(['art_chart'])
  })
})
