/// Material 3 window size classes.
///
/// The numbers are the spec's, in device-independent pixels, and they are the
/// only thing that decides which navigation component is on screen. Inventing
/// a breakpoint that "looks better on this laptop" is how an adaptive layout
/// stops being adaptive.
///
/// https://m3.material.io/foundations/layout/applying-layout
enum WindowSizeClass {
  compact,
  medium,
  expanded,
  large,
  extraLarge;

  static WindowSizeClass of(double width) {
    if (width < 600) return WindowSizeClass.compact;
    if (width < 840) return WindowSizeClass.medium;
    if (width < 1200) return WindowSizeClass.expanded;
    if (width < 1600) return WindowSizeClass.large;
    return WindowSizeClass.extraLarge;
  }
}
