#include "helpers.h"
#include <QClipboard>
#include <QDir>
#include <QFileDevice>
#include <QGuiApplication>
#include <QImage>
#include <QTemporaryFile>
#include <QtWebEngineQuick/qtwebenginequickglobal.h>

extern "C" void impulse_init_webengine() {
    QtWebEngineQuick::initialize();
}

QString impulse_clipboard_image_to_temp_png() {
    auto *app = QGuiApplication::instance();
    if (!app) {
        return {};
    }

    auto *clipboard = QGuiApplication::clipboard();
    if (!clipboard) {
        return {};
    }

    const QImage image = clipboard->image();
    if (image.isNull()) {
        return {};
    }

    QTemporaryFile file(QDir::tempPath() + "/impulse-clipboard-XXXXXX.png");
    file.setAutoRemove(false);
    if (!file.open()) {
        return {};
    }

    if (!image.save(&file, "PNG")) {
        return {};
    }

    file.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner);
    file.flush();
    return file.fileName();
}
