import 'package:flutter/material.dart';

/// The seed the whole palette is generated from.
///
/// `#2C3E50` is the accent the TypeScript UI uses (`ui/src/style.css`), so the
/// two frontends read as one product. It is the only colour named in this
/// file on purpose: Material 3 derives every surface, container and "on" role
/// from a seed, and hand-picking those roles is exactly what the spec exists
/// to stop.
const Color _seed = Color(0xFF2C3E50);

/// Material 3 is the default in this Flutter version; `useMaterial3` is not
/// set here because setting it could only turn the spec *off*.
ThemeData lightTheme() => _theme(Brightness.light);

ThemeData darkTheme() => _theme(Brightness.dark);

ThemeData _theme(Brightness brightness) {
  final ColorScheme colors = ColorScheme.fromSeed(
    seedColor: _seed,
    brightness: brightness,
  );
  return ThemeData(colorScheme: colors);
}
