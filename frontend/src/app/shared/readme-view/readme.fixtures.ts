export const RICH_README_HTML = `
<h1>@acme/ui</h1>
<p>A small set of <strong>accessible</strong> components. See the <a href="https://example.com/docs" target="_blank" rel="nofollow noopener noreferrer ugc">documentation</a> or run <code>npm test</code>.</p>
<h2>Installation</h2>
<pre><code>npm install @acme/ui
</code></pre>
<h2>Usage</h2>
<ul><li>Import the module</li><li>Add <code>&lt;acme-button&gt;</code> to a template</li></ul>
<blockquote><p>Requires Node 20 or later.</p></blockquote>
<h3>Options</h3>
<table>
<thead><tr><th>Name</th><th>Type</th><th>Default</th></tr></thead>
<tbody>
<tr><td>size</td><td>string</td><td>medium</td></tr>
<tr><td>disabled</td><td>boolean</td><td>false</td></tr>
</tbody>
</table>
<ol><li>First</li><li>Second</li></ol>
`

const LONG_LINE =
  'const veryLongIdentifierName = callSomething(argumentNumberOne, argumentNumberTwo, argumentNumberThree, argumentNumberFour, argumentNumberFive);'

const PIXEL_PNG =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=='

/** Content that tends to blow the layout out: unbreakable strings, wide code, tables and images. */
export const OVERFLOW_README_HTML = `
<p>https://example.com/a/very/long/url/that/has/no/spaces/and/keeps/going/and/going/and/going/and/going/and/going/forever</p>
<p>${'Averylongunbrokenwordwithoutanyspaceatall'.repeat(6)}</p>
<pre><code>${LONG_LINE}</code></pre>
<table>
<thead><tr><th>Column one</th><th>Column two</th><th>Column three</th><th>Column four</th><th>Column five</th><th>Column six</th><th>Column seven</th><th>Column eight</th></tr></thead>
<tbody><tr><td>${'wide cell content '.repeat(4)}</td><td>b</td><td>c</td><td>d</td><td>e</td><td>f</td><td>g</td><td>${'another wide cell '.repeat(4)}</td></tr></tbody>
</table>
<p><img src="${PIXEL_PNG}" alt="A very wide image" width="2400" height="60"></p>
`
