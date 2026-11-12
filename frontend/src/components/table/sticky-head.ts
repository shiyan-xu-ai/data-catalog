/** Header cell of a plain table (a scroll wrapper would scope `sticky` to itself) that pins below
 *  the tab bar and `--sticky-top`, the height of any pinned element above the table. The 1px
 *  overlap hides sub-pixel seams at fractional zoom; the top rule closes the card's border once
 *  its top scrolls away. */
export const STICKY_HEAD =
  "sticky top-[calc(var(--tabs-h,0px)+var(--sticky-top,0px)-1px)] z-10 h-10 bg-muted px-2 text-left align-middle text-sm font-medium whitespace-nowrap text-muted-foreground shadow-[inset_0_1px_0_hsl(var(--border)),inset_0_-1px_0_hsl(var(--border))]";
