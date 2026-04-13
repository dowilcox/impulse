#pragma once

#include <QtCore/QString>

#ifdef __cplusplus
extern "C" {
#endif

/// Initialize QtWebEngine before QGuiApplication creation.
void impulse_init_webengine();

#ifdef __cplusplus
}
#endif

/// Save the current clipboard image to a temp PNG and return its path.
/// Returns an empty string when the clipboard does not contain an image.
QString impulse_clipboard_image_to_temp_png();
