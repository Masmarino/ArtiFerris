import { csvBlob, toCsv } from './csv'

describe('toCsv', () => {
  it('renders a header row followed by one row per item', () => {
    const csv = toCsv(
      [
        { name: 'alice', age: 30 },
        { name: 'bob', age: 25 },
      ],
      [
        { key: 'name', label: 'Name' },
        { key: 'age', label: 'Age' },
      ],
    )

    expect(csv).toBe('Name;Age\r\nalice;30\r\nbob;25')
  })

  it('quotes a value containing the separator, and leaves a comma alone', () => {
    const csv = toCsv([{ label: 'a; b' }, { label: 'a, b' }], [{ key: 'label', label: 'Label' }])

    expect(csv).toBe('Label\r\n"a; b"\r\na, b')
  })

  it('escapes an embedded double quote by doubling it', () => {
    const csv = toCsv([{ label: 'say "hi"' }], [{ key: 'label', label: 'Label' }])

    expect(csv).toBe('Label\r\n"say ""hi"""')
  })

  it('renders null and undefined values as an empty cell', () => {
    const csv = toCsv([{ value: null }, { value: undefined }], [{ key: 'value', label: 'Value' }])

    expect(csv).toBe('Value\r\n\r\n')
  })

  it('produces just the header row for an empty array', () => {
    const csv = toCsv([], [{ key: 'name', label: 'Name' }])

    expect(csv).toBe('Name')
  })

  it('neutralizes a leading =, +, - or @ so the cell can never be read as a formula', () => {
    const csv = toCsv(
      [{ v: '=cmd|/c calc' }, { v: '+1' }, { v: '-x' }, { v: '@SUM(A1)' }],
      [{ key: 'v', label: 'V' }],
    )

    expect(csv).toBe("V\r\n'=cmd|/c calc\r\n'+1\r\n'-x\r\n'@SUM(A1)")
  })

  it('neutralizes a leading tab', () => {
    expect(toCsv([{ v: '\t=1+1' }], [{ key: 'v', label: 'V' }])).toBe("V\r\n'\t=1+1")
  })

  it('keeps a legitimate negative number readable', () => {
    const csv = toCsv([{ v: -5 }, { v: '-12.5' }, { v: '-1+2' }], [{ key: 'v', label: 'V' }])

    expect(csv).toBe("V\r\n-5\r\n-12.5\r\n'-1+2")
  })

  it('neutralizes a formula after a lone CR and quotes the field', () => {
    const csv = toCsv([{ v: 'a\r=HYPERLINK("http://evil","x")' }], [{ key: 'v', label: 'V' }])

    expect(csv).toBe('V\r\n"a\n\'=HYPERLINK(""http://evil"",""x"")"')
    expect(csv).not.toMatch(/\r(?!\n)/)
  })

  it('neutralizes a formula after every line break of a field, CRLF and LF included', () => {
    const csv = toCsv([{ v: 'a\r\n+1\n@b\r\tc' }], [{ key: 'v', label: 'V' }])

    expect(csv).toBe("V\r\n\"a\n'+1\n'@b\n'\tc\"")
  })

  it('neutralizes a formula in a column label', () => {
    expect(toCsv([], [{ key: 'v', label: '=1' }])).toBe("'=1")
  })

  it('still quotes a neutralized value that also contains the separator', () => {
    const csv = toCsv([{ v: '=SUM(A1;B1)' }], [{ key: 'v', label: 'V' }])

    expect(csv).toBe('V\r\n"\'=SUM(A1;B1)"')
  })

  it('does not touch a value that merely contains one of those characters mid-string', () => {
    const csv = toCsv([{ v: 'a=b' }, { v: 'x-y' }], [{ key: 'v', label: 'V' }])

    expect(csv).toBe('V\r\na=b\r\nx-y')
  })
})

describe('csvBlob', () => {
  it('starts with a UTF-8 BOM', async () => {
    const bytes = new Uint8Array(await csvBlob('é').arrayBuffer())

    expect([...bytes.slice(0, 3)]).toEqual([0xef, 0xbb, 0xbf])
    expect(csvBlob('é').type).toBe('text/csv;charset=utf-8')
  })
})
