#include "main_window.h"

#include <QApplication>
#include <QLabel>
#include <QShortcut>
#include <QStackedLayout>
#include <QVBoxLayout>

#ifdef REACTOR_OS_USE_WEBENGINE
#include <QWebEngineProfile>
#include <QWebEngineSettings>
#include <QWebEngineView>
#else
#include <QWebSettings>
#include <QWebView>
#endif

MainWindow::MainWindow(const QUrl &url, bool fullscreen, const QString &backendCommand,
                       QWidget *parent)
    : QMainWindow(parent),
      url_(url),
      fullscreen_(fullscreen),
      backendCommand_(backendCommand),
#ifdef REACTOR_OS_USE_WEBENGINE
      view_(new QWebEngineView(this)),
      webEngineView_(static_cast<QWebEngineView *>(view_)),
#else
      view_(new QWebView(this)),
      webKitView_(static_cast<QWebView *>(view_)),
#endif
      overlay_(new QLabel(this)) {
    setWindowTitle(QStringLiteral("ReactorOS HMI"));
    setMinimumSize(1024, 600);

    auto *root = new QWidget(this);
    auto *layout = new QStackedLayout(root);
    layout->setStackingMode(QStackedLayout::StackAll);
    layout->setContentsMargins(0, 0, 0, 0);

#ifdef REACTOR_OS_USE_WEBENGINE
    webEngineView_->settings()->setAttribute(QWebEngineSettings::FullScreenSupportEnabled, true);
    webEngineView_->settings()->setAttribute(QWebEngineSettings::LocalStorageEnabled, true);
    webEngineView_->page()->profile()->setHttpCacheType(QWebEngineProfile::MemoryHttpCache);
    connect(webEngineView_, &QWebEngineView::loadStarted, this, &MainWindow::handleLoadStarted);
    connect(webEngineView_, &QWebEngineView::loadFinished, this, &MainWindow::handleLoadFinished);
#else
    webKitView_->settings()->setAttribute(QWebSettings::LocalStorageEnabled, true);
    webKitView_->settings()->setAttribute(QWebSettings::JavascriptEnabled, true);
    webKitView_->settings()->setAttribute(QWebSettings::DeveloperExtrasEnabled, false);
    connect(webKitView_, &QWebView::loadStarted, this, &MainWindow::handleLoadStarted);
    connect(webKitView_, &QWebView::loadFinished, this, &MainWindow::handleLoadFinished);
#endif

    overlay_->setAlignment(Qt::AlignCenter);
    overlay_->setStyleSheet(
        "QLabel {"
        "background: #FAF8F5;"
        "color: #1A1814;"
        "font-family: sans-serif;"
        "font-size: 20px;"
        "letter-spacing: 1px;"
        "}");
    overlay_->hide();

    layout->addWidget(view_);
    layout->addWidget(overlay_);
    setCentralWidget(root);

    retryTimer_.setInterval(2000);
    retryTimer_.setSingleShot(false);
    connect(&retryTimer_, &QTimer::timeout, this, &MainWindow::reload);
    connect(&backend_, QOverload<int, QProcess::ExitStatus>::of(&QProcess::finished), this,
            &MainWindow::handleBackendFinished);

    auto *reloadShortcut = new QShortcut(QKeySequence(QStringLiteral("F5")), this);
    connect(reloadShortcut, &QShortcut::activated, this, &MainWindow::reload);
    auto *quitShortcut = new QShortcut(QKeySequence(QStringLiteral("Ctrl+Q")), this);
    connect(quitShortcut, &QShortcut::activated, qApp, &QApplication::quit);
    auto *fullscreenShortcut = new QShortcut(QKeySequence(QStringLiteral("F11")), this);
    connect(fullscreenShortcut, &QShortcut::activated, this, [this]() {
        isFullScreen() ? showNormal() : showFullScreen();
    });

    startBackendIfRequested();
    reload();

    fullscreen_ ? showFullScreen() : show();
}

MainWindow::~MainWindow() {
    if (backend_.state() != QProcess::NotRunning) {
        backend_.terminate();
        if (!backend_.waitForFinished(3000)) {
            backend_.kill();
            backend_.waitForFinished(1000);
        }
    }
}

void MainWindow::reload() {
    showOverlay(QStringLiteral("Connecting to ReactorOS..."));
#ifdef REACTOR_OS_USE_WEBENGINE
    webEngineView_->load(url_);
#else
    webKitView_->load(url_);
#endif
}

void MainWindow::handleLoadStarted() {
    showOverlay(QStringLiteral("Loading HMI..."));
}

void MainWindow::handleLoadFinished(bool ok) {
    if (ok) {
        retryTimer_.stop();
        hideOverlay();
        return;
    }

    showOverlay(QStringLiteral("ReactorOS backend unavailable. Retrying..."));
    if (!retryTimer_.isActive()) {
        retryTimer_.start();
    }
}

void MainWindow::handleBackendFinished(int exitCode, QProcess::ExitStatus exitStatus) {
    Q_UNUSED(exitStatus);
    showOverlay(QStringLiteral("ReactorOS backend exited. Code %1").arg(exitCode));
}

void MainWindow::startBackendIfRequested() {
    if (backendCommand_.trimmed().isEmpty()) {
        return;
    }

    backend_.setProcessChannelMode(QProcess::ForwardedChannels);
    backend_.start(QStringLiteral("/bin/sh"), QStringList() << QStringLiteral("-lc") << backendCommand_);
}

void MainWindow::showOverlay(const QString &message) {
    overlay_->setText(message);
    overlay_->show();
    overlay_->raise();
}

void MainWindow::hideOverlay() {
    overlay_->hide();
}
