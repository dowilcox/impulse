#include "helpers.h"
#include <QtWebEngineQuick/qtwebenginequickglobal.h>

extern "C" void impulse_init_webengine() {
    QtWebEngineQuick::initialize();
}
