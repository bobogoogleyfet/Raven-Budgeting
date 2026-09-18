{{flutter_js}}
{{flutter_build_config}}

// Replaces the bootstrap Flutter would otherwise generate, for two reasons.
//
// Fonts. `--no-web-resources-cdn` bundles CanvasKit and Roboto into the build,
// but a character Roboto lacks — CJK, emoji, most scripts that are not Latin,
// Greek or Cyrillic — still sends the engine to fonts.gstatic.com for a Noto
// font. That is a request to Google carrying the visitor's address, from a
// page that promises to talk only to its own origin (and whose
// Content-Security-Policy would refuse it anyway). So the fallback base URL
// points at this site instead, where no such fonts exist: the engine gets a
// 404, gives up on fallback fonts for the session, and draws a box. That is
// the price of not asking Google, and a merchant name in Japanese is the only
// place anybody will ever pay it.
//
// No service worker. The default bootstrap registers Flutter's, which is
// deprecated; caching an app whose data lives in IndexedDB buys little and
// makes every release a question of which version a tab is running.
_flutter.loader.load({
  config: {
    fontFallbackBaseUrl: 'fonts/',
  },
});
