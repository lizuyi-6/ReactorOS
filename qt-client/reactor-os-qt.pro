QT += core gui widgets

WEB_BACKEND = $$(REACTOR_OS_QT_BACKEND)
isEmpty(WEB_BACKEND) {
    qtHaveModule(webenginewidgets) {
        WEB_BACKEND = webengine
    } else: qtHaveModule(webkitwidgets) {
        WEB_BACKEND = webkit
    } else {
        error("No Qt web view module found. Install qtwebengine5-dev or libqt5webkit5-dev.")
    }
}

equals(WEB_BACKEND, webkit) {
    QT += webkitwidgets
    DEFINES += REACTOR_OS_USE_WEBKIT
    message("ReactorOS Qt backend: WebKit")
} else: equals(WEB_BACKEND, webengine) {
    QT += webenginewidgets
    DEFINES += REACTOR_OS_USE_WEBENGINE
    message("ReactorOS Qt backend: WebEngine")
} else {
    error("Unsupported REACTOR_OS_QT_BACKEND. Use webengine or webkit.")
}

CONFIG += c++17
CONFIG -= app_bundle

TARGET = reactor-os-qt
TEMPLATE = app

SOURCES += \
    src/main.cpp \
    src/main_window.cpp

HEADERS += \
    src/main_window.h
