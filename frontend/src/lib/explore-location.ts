let last = "/namespaces";

/** The most recent location outside the Query page, so switching back from SQL mode returns
 *  to it. Kept in memory only. */
export const rememberExploreLocation = (href: string): void => {
  last = href;
};

export const lastExploreLocation = (): string => last;
