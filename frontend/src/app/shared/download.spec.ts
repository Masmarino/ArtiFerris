import { downloadBlob } from './download'

describe('downloadBlob', () => {
  const original = { create: URL.createObjectURL, revoke: URL.revokeObjectURL }

  afterEach(() => {
    URL.createObjectURL = original.create
    URL.revokeObjectURL = original.revoke
    vi.useRealTimers()
    vi.restoreAllMocks()
  })

  it('clicks a download link, and only revokes the object URL afterwards', () => {
    vi.useFakeTimers()
    const revoke = vi.fn()
    URL.createObjectURL = vi.fn().mockReturnValue('blob:x')
    URL.revokeObjectURL = revoke
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined)

    downloadBlob(new Blob(['a']), 'a.csv')

    expect(click).toHaveBeenCalledTimes(1)
    expect(revoke).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1000)
    expect(revoke).toHaveBeenCalledWith('blob:x')
  })
})
