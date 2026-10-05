import { FormatBytesPipe } from './format-bytes.pipe'

describe('FormatBytesPipe', () => {
  const pipe = new FormatBytesPipe()

  it('formats a size', () => {
    expect(pipe.transform(1536)).toBe('1,5 Ko')
  })

  it('shows a dash when the size is unknown', () => {
    expect(pipe.transform(null)).toBe('—')
    expect(pipe.transform(undefined)).toBe('—')
  })
})
