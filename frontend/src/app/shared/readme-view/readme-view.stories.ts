import { componentWrapperDecorator, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, within } from 'storybook/test'
import { OVERFLOW_README_HTML, RICH_README_HTML } from './readme.fixtures'
import { ReadmeView } from './readme-view'

const meta: Meta<ReadmeView> = {
  title: 'Shared/ReadmeView',
  component: ReadmeView,
  args: { html: RICH_README_HTML },
}
export default meta

type Story = StoryObj<ReadmeView>

/** Headings, links, inline and block code, quote, lists and a table. */
export const RichReadme: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByRole('heading', { name: 'README', level: 2 })).toBeInTheDocument()
    expect(canvas.getByRole('heading', { name: 'Usage' })).toBeInTheDocument()
    expect(canvas.getByRole('link', { name: 'documentation' })).toHaveAttribute('target', '_blank')
    expect(canvas.getByRole('table')).toBeInTheDocument()
    expect(canvasElement.querySelector('pre code')).toHaveTextContent('npm install @acme/ui')
  },
}

/** No README: a discreet empty state under the same heading. */
export const NoReadme: Story = {
  args: { html: null },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByText('Aucun README')).toBeInTheDocument()
    expect(canvasElement.querySelector('.readme-view__content')).toBeNull()
  },
}

/** Long strings wrap, code and tables scroll inside the card, images shrink to fit. */
export const OverflowingContent: Story = {
  args: { html: OVERFLOW_README_HTML },
  decorators: [
    componentWrapperDecorator((story) => `<div style="max-width: 360px">${story}</div>`),
  ],
  play: async ({ canvasElement }) => {
    const card = canvasElement.querySelector<HTMLElement>('.readme-view')!
    const image = canvasElement.querySelector<HTMLElement>('img')!
    expect(image.getBoundingClientRect().right).toBeLessThanOrEqual(
      card.getBoundingClientRect().right,
    )
    const pre = canvasElement.querySelector<HTMLElement>('pre')!
    expect(pre.scrollWidth).toBeGreaterThan(pre.clientWidth)
    expect(card.scrollWidth).toBeLessThanOrEqual(card.clientWidth)
  },
}
