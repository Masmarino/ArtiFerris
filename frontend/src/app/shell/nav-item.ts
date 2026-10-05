/** An entry of the rail, which the quick search offers too: what one sees there is what one can jump to. */
export interface NavItem {
  action: string
  icon: string
  text: string
  link: string
  children?: NavItem[]
  /** '/' would contain every route under non-exact matching, so it always lit up. */
  exact?: boolean
}
