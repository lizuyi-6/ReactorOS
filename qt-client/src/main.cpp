#include "main_window.h"

#include <QApplication>
#include <QCommandLineParser>
#include <QCoreApplication>
#include <QUrl>

int main(int argc, char *argv[]) {
    QCoreApplication::setAttribute(Qt::AA_EnableHighDpiScaling);
    QCoreApplication::setAttribute(Qt::AA_UseHighDpiPixmaps);

    QApplication app(argc, argv);
    QApplication::setApplicationName(QStringLiteral("ReactorOS HMI"));
    QApplication::setApplicationVersion(QStringLiteral("1.0"));

    QCommandLineParser parser;
    parser.setApplicationDescription(QStringLiteral("Qt shell for the ReactorOS industrial HMI"));
    parser.addHelpOption();
    parser.addVersionOption();

    QCommandLineOption urlOption(QStringList() << QStringLiteral("u") << QStringLiteral("url"),
                                 QStringLiteral("HMI URL to load."),
                                 QStringLiteral("url"),
                                 QStringLiteral("http://127.0.0.1:8000/"));
    QCommandLineOption windowedOption(QStringList() << QStringLiteral("windowed"),
                                      QStringLiteral("Run in a normal window instead of fullscreen."));
    QCommandLineOption backendOption(QStringList() << QStringLiteral("backend"),
                                     QStringLiteral("Optional backend command to start before loading the UI."),
                                     QStringLiteral("command"));
    parser.addOption(urlOption);
    parser.addOption(windowedOption);
    parser.addOption(backendOption);
    parser.process(app);

    const QUrl url = QUrl::fromUserInput(parser.value(urlOption));
    const bool fullscreen = !parser.isSet(windowedOption);
    const QString backendCommand = parser.value(backendOption);

    MainWindow window(url, fullscreen, backendCommand);
    return app.exec();
}
