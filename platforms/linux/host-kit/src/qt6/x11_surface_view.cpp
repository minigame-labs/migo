#include <migo/linux/qt6/x11_surface_view.hpp>

#include <QCloseEvent>
#include <QGuiApplication>
#include <QPaintEngine>
#include <QPlatformSurfaceEvent>
#include <QPointer>
#include <QResizeEvent>
#include <QScreen>
#include <QShowEvent>
#include <QThread>
#include <QWindow>
#include <QtGui/qguiapplication_platform.h>

#if !QT_CONFIG(xcb)
#error "MigoQtX11SurfaceView requires a Qt build with the xcb platform feature"
#endif

#include <X11/Xlib.h>

#include <cmath>
#include <limits>

namespace migo::linux_host::qt6 {

namespace {

constexpr int kFastReleasePollIntervalMs = 4;
constexpr int kNormalReleasePollIntervalMs = 16;
constexpr int kStalledReleasePollIntervalMs = 250;
constexpr qint64 kFastReleaseWindowMs = 100;
constexpr qint64 kReleaseStallThresholdMs = 2000;

}  // namespace

MigoQtX11SurfaceView::MigoQtX11SurfaceView(SurfaceHost &surface_host, QWidget &parent)
    : QWidget(&parent),
      surface_host_(surface_host),
      owner_thread_(QThread::currentThread()),
      release_timer_(this),
      metrics_update_timer_(this) {
    setAttribute(Qt::WA_DontCreateNativeAncestors);
    setAttribute(Qt::WA_NativeWindow);
    setAttribute(Qt::WA_PaintOnScreen);
    setAttribute(Qt::WA_OpaquePaintEvent);
    setAttribute(Qt::WA_NoSystemBackground);
    release_timer_.setInterval(kFastReleasePollIntervalMs);
    release_timer_.setTimerType(Qt::PreciseTimer);
    connect(&release_timer_, &QTimer::timeout, this, [this] {
        bool released = false;
        (void)pollDetach(&released);
    });

    metrics_update_timer_.setSingleShot(true);
    connect(&metrics_update_timer_, &QTimer::timeout, this, [this] {
        if (!owns_surface_ || surface_host_.state() != SurfaceState::Attached ||
            surface_host_.generation() != owned_generation_) {
            return;
        }
        SurfaceMetrics metrics;
        const MigoResult metrics_result = currentMetrics(&metrics);
        if (metrics_result != MIGO_OK) {
            recordError(metrics_result);
            return;
        }
        recordError(surface_host_.update(metrics));
    });
}

MigoQtX11SurfaceView::~MigoQtX11SurfaceView() {
    release_timer_.stop();
    metrics_update_timer_.stop();
    if (owns_surface_) {
        qFatal("MigoQtX11SurfaceView destroyed before its native Surface reached RELEASED");
    }
}

QPaintEngine *MigoQtX11SurfaceView::paintEngine() const { return nullptr; }

MigoResult MigoQtX11SurfaceView::currentMetrics(SurfaceMetrics *metrics) const noexcept {
    if (metrics == nullptr) return MIGO_ERROR_INVALID_ARGUMENT;
    const double scale = devicePixelRatioF();
    const double physical_width = static_cast<double>(width()) * scale;
    const double physical_height = static_cast<double>(height()) * scale;
    constexpr double kMaxDimension =
        static_cast<double>(std::numeric_limits<std::uint32_t>::max());
    if (!std::isfinite(scale) || scale <= 0.0 || physical_width < 1.0 ||
        physical_height < 1.0 || physical_width > kMaxDimension ||
        physical_height > kMaxDimension) {
        return MIGO_ERROR_INVALID_ARGUMENT;
    }

    metrics->width_pixels = static_cast<std::uint32_t>(std::llround(physical_width));
    metrics->height_pixels = static_cast<std::uint32_t>(std::llround(physical_height));
    metrics->scale_factor = static_cast<float>(scale);
    metrics->color_space = MIGO_COLOR_SPACE_SRGB;
    metrics->alpha_mode = MIGO_ALPHA_MODE_OPAQUE;
    metrics->presentation_mode = MIGO_PRESENTATION_MODE_DEFAULT;
    metrics->required_capabilities = MIGO_SURFACE_CAPABILITY_NONE;
    return MIGO_OK;
}

void MigoQtX11SurfaceView::recordError(MigoResult error) {
    last_error_ = error;
    if (error != MIGO_OK) Q_EMIT surfaceError(error);
}

MigoResult MigoQtX11SurfaceView::attachSurface() {
    if (QThread::currentThread() != owner_thread_) return MIGO_ERROR_WRONG_THREAD;
    if (owns_surface_ || surface_host_.state() != SurfaceState::Detached || close_pending_) {
        recordError(MIGO_ERROR_INVALID_STATE);
        return MIGO_ERROR_INVALID_STATE;
    }
    if (QGuiApplication::platformName() != QStringLiteral("xcb")) {
        recordError(MIGO_ERROR_UNSUPPORTED_PLATFORM);
        return MIGO_ERROR_UNSUPPORTED_PLATFORM;
    }

    auto *native = qGuiApp->nativeInterface<QNativeInterface::QX11Application>();
    if (native == nullptr || native->display() == nullptr) {
        recordError(MIGO_ERROR_UNSUPPORTED_PLATFORM);
        return MIGO_ERROR_UNSUPPORTED_PLATFORM;
    }

    const WId id = winId();
    if (id == 0) {
        recordError(MIGO_ERROR_INTERNAL);
        return MIGO_ERROR_INTERNAL;
    }

    // A detached QWidget may be reparented, which can replace its QWindow and
    // silently invalidate the old sender connection. Rebuild this cheap
    // control-path connection on every attach instead of caching a boolean.
    QObject::disconnect(screen_changed_connection_);
    screen_changed_connection_ = {};
    if (QWindow *native_window = windowHandle(); native_window != nullptr) {
        screen_changed_connection_ =
            connect(native_window, &QWindow::screenChanged, this,
                    [this](QScreen *) { scheduleMetricsUpdate(); });
    }

    XWindowAttributes attributes{};
    const ::Window x11_window = static_cast<::Window>(id);
    if (XGetWindowAttributes(native->display(), x11_window, &attributes) == 0 ||
        attributes.screen == nullptr) {
        recordError(MIGO_ERROR_INTERNAL);
        return MIGO_ERROR_INTERNAL;
    }

    SurfaceMetrics metrics;
    const MigoResult metrics_result = currentMetrics(&metrics);
    if (metrics_result != MIGO_OK) {
        recordError(metrics_result);
        return metrics_result;
    }

    const X11Target target{native->display(), static_cast<std::uintptr_t>(id),
                           XScreenNumberOfScreen(attributes.screen)};
    const MigoResult result = surface_host_.attach(target, metrics);
    if (result == MIGO_OK || surface_host_.state() == SurfaceState::Faulted) {
        owns_surface_ = true;
        owned_generation_ = surface_host_.generation();
    }
    recordError(result);
    if (result == MIGO_OK) Q_EMIT surfaceAttached(owned_generation_);
    return result;
}

MigoResult MigoQtX11SurfaceView::beginDetach() {
    if (QThread::currentThread() != owner_thread_) return MIGO_ERROR_WRONG_THREAD;
    if (!owns_surface_ || surface_host_.state() != SurfaceState::Attached ||
        surface_host_.generation() != owned_generation_) {
        recordError(MIGO_ERROR_INVALID_STATE);
        return MIGO_ERROR_INVALID_STATE;
    }
    const MigoResult result = surface_host_.begin_detach();
    recordError(result);
    if (result == MIGO_OK) {
        metrics_update_timer_.stop();
        release_stall_reported_ = false;
        release_elapsed_.start();
        release_timer_.setInterval(kFastReleasePollIntervalMs);
        release_timer_.start();
    }
    return result;
}

MigoResult MigoQtX11SurfaceView::pollDetach(bool *released) {
    if (QThread::currentThread() != owner_thread_) return MIGO_ERROR_WRONG_THREAD;
    if (released == nullptr) {
        recordError(MIGO_ERROR_INVALID_ARGUMENT);
        return MIGO_ERROR_INVALID_ARGUMENT;
    }
    if (!owns_surface_ || surface_host_.state() != SurfaceState::Retiring ||
        surface_host_.generation() != owned_generation_) {
        recordError(MIGO_ERROR_INVALID_STATE);
        return MIGO_ERROR_INVALID_STATE;
    }
    const MigoResult result = surface_host_.poll_release(released);
    if (result != MIGO_OK) {
        release_timer_.stop();
        recordError(result);
        return result;
    }
    recordError(result);
    if (!*released) {
        updateReleasePollCadence();
        return result;
    }

    release_timer_.stop();
    release_elapsed_.invalidate();
    const std::uint64_t released_generation = owned_generation_;
    owned_generation_ = 0;
    owns_surface_ = false;
    const bool should_close = close_pending_;
    QPointer<MigoQtX11SurfaceView> guard(this);
    Q_EMIT surfaceReleased(released_generation);
    if (guard != nullptr) {
        guard->close_pending_ = false;
        if (should_close) guard->close();
    }
    return MIGO_OK;
}

void MigoQtX11SurfaceView::updateReleasePollCadence() {
    if (!release_elapsed_.isValid()) return;
    const qint64 elapsed_ms = release_elapsed_.elapsed();
    int interval_ms = kFastReleasePollIntervalMs;
    if (elapsed_ms >= kReleaseStallThresholdMs) {
        interval_ms = kStalledReleasePollIntervalMs;
    } else if (elapsed_ms >= kFastReleaseWindowMs) {
        interval_ms = kNormalReleasePollIntervalMs;
    }
    if (release_timer_.interval() != interval_ms) release_timer_.setInterval(interval_ms);
    if (!release_timer_.isActive()) release_timer_.start();

    if (elapsed_ms >= kReleaseStallThresholdMs && !release_stall_reported_) {
        release_stall_reported_ = true;
        Q_EMIT surfaceReleaseStalled(surface_host_.generation(), elapsed_ms);
    }
}

void MigoQtX11SurfaceView::scheduleMetricsUpdate() {
    if (owns_surface_ && surface_host_.state() == SurfaceState::Attached &&
        surface_host_.generation() == owned_generation_ &&
        !metrics_update_timer_.isActive()) {
        metrics_update_timer_.start(0);
    }
}

void MigoQtX11SurfaceView::showEvent(QShowEvent *event) {
    QWidget::showEvent(event);
    if (!owns_surface_ && surface_host_.state() == SurfaceState::Detached &&
        !close_pending_) {
        (void)attachSurface();
    }
}

void MigoQtX11SurfaceView::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    scheduleMetricsUpdate();
}

void MigoQtX11SurfaceView::closeEvent(QCloseEvent *event) {
    if (!owns_surface_) {
        close_pending_ = false;
        QWidget::closeEvent(event);
        return;
    }
    switch (surface_host_.state()) {
        case SurfaceState::Detached:
            qFatal("MigoQtX11SurfaceView attachment was released outside its owner view");
        case SurfaceState::Attached:
            event->ignore();
            close_pending_ = true;
            {
                QPointer<MigoQtX11SurfaceView> guard(this);
                const MigoResult result = beginDetach();
                if (guard != nullptr && result != MIGO_OK) guard->close_pending_ = false;
            }
            return;
        case SurfaceState::Retiring:
            close_pending_ = true;
            event->ignore();
            return;
        case SurfaceState::Faulted:
            qFatal("MigoQtX11SurfaceView cannot close after losing its release observer");
    }
}

bool MigoQtX11SurfaceView::event(QEvent *event) {
    if (owns_surface_ && event->type() == QEvent::PlatformSurface) {
        const auto *surface_event = static_cast<QPlatformSurfaceEvent *>(event);
        if (surface_event->surfaceEventType() ==
                QPlatformSurfaceEvent::SurfaceAboutToBeDestroyed &&
            surface_host_.state() != SurfaceState::Detached) {
            qFatal("Qt attempted to destroy an X11 Surface before Migo reported RELEASED");
        }
    }
    return QWidget::event(event);
}

}  // namespace migo::linux_host::qt6
