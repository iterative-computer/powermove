import { copyData } from "./data.mjs";

/** WebKit render-side adapter, called with the already-created panel Document.
 * Keep extension logic/events in V8. afterReplace only reports render completion;
 * the kernel's existing theme:changed bus remains authoritative.
 * Insert before panel styles; mutate retained style nodes without navigation.
 */
export function createWebKitThemeHost(panelDocument, afterReplace = () => {}) {
  const root = panelDocument.documentElement;
  const foundation = panelDocument.createElement("style");
  foundation.setAttribute("data-powermove-compat-edition", "1");
  const theme = panelDocument.createElement("style");
  theme.setAttribute("data-powermove-theme", "");
  let attributes = [],
    tokens = [],
    mounted = false;
  return {
    replaceTheme(edition) {
      // Validate attribute names with this DOM before touching a working theme.
      for (const key of Object.keys(edition.rootAttributes))
        panelDocument.createAttribute(key);
      // Same JS task: observers/paint see the complete replacement. textContent
      // avoids HTML parsing, including authored </style> strings in CSS values.
      if (!mounted) {
        panelDocument.head.insertBefore(
          foundation,
          panelDocument.head.firstChild,
        );
        panelDocument.head.appendChild(theme);
        mounted = true;
      }
      for (const key of tokens) root.style.removeProperty(key);
      for (const key of attributes) root.removeAttribute(key);
      foundation.textContent = edition.stylesheet;
      theme.textContent = edition.themeCss;
      for (const [key, value] of Object.entries(edition.rootAttributes))
        root.setAttribute(key, value);
      for (const [key, value] of Object.entries(edition.inlineTokens))
        root.style.setProperty(key, value);
      attributes = Object.keys(edition.rootAttributes);
      tokens = Object.keys(edition.inlineTokens);
      afterReplace(
        copyData({
          id: edition.themeId,
          scheme: edition.scheme,
          edition: edition.id,
        }),
      );
    },
  };
}
