#pragma once

#include <QMainWindow>
#include <QProcess>
#include <QTimer>
#include <QUrl>

class QWebEngineView;
class QWebView;
class QLabel;
class QShortcut;
class QWidget;

class MainWindow final : public QMainWindow {
    Q_OBJECT

public:
    explicit MainWindow(const QUrl &url, bool fullscreen, const QString &backendCommand,
                        QWidget *parent = nullptr);
    ~MainWindow() override;

private slots:
    void reload();
    void handleLoadStarted();
    void handleLoadFinished(bool ok);
    void handleBackendFinished(int exitCode, QProcess::ExitStatus exitStatus);

private:
    void startBackendIfRequested();
    void showOverlay(const QString &message);
    void hideOverlay();

    QUrl url_;
    bool fullscreen_;
    QString backendCommand_;
    QWidget *view_;
#ifdef REACTOR_OS_USE_WEBENGINE
    QWebEngineView *webEngineView_;
#else
    QWebView *webKitView_;
#endif
    QLabel *overlay_;
    QTimer retryTimer_;
    QProcess backend_;
};
