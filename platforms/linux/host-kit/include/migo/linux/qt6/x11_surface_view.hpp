#ifndef MIGO_LINUX_QT6_X11_SURFACE_VIEW_HPP_
#define MIGO_LINUX_QT6_X11_SURFACE_VIEW_HPP_

#include <migo/linux/surface_host.hpp>

#include <QElapsedTimer>
#include <QMetaObject>
#include <QTimer>
#include <QWidget>

class QCloseEvent;
class QPaintEngine;
class QResizeEvent;
class QShowEvent;
class QThread;

namespace migo::linux_host::qt6 {

/// A surface-only Qt 6 Widgets adapter for an App-owned SurfaceHost.
///
/// The widget is a native X11 child window placed by the host's layout. It does
/// not create/configure the Session, install its callback table, load content,
/// or translate input. Those belong to the future Managed Host Kit and to the
/// host that selected Bound ownership. Call `close()` (or `beginDetach()` and
/// `pollDetach()`) and wait for `surfaceReleased` before destroying a parent
/// that owns this widget's native window. SurfaceHost and QApplication must
/// each outlive this view. The controller remains at a stable address; keeping
/// it at Session scope preserves
/// the C ABI's strictly increasing Surface generation across replacement views.
/// Only one view may be Attached or Retiring through that controller at once.
/// State/generation accessors are local to this view: a passive replacement
/// never updates, retires, or fail-fast destroys another view's attachment.
/// Once this view attaches, drive that generation through the view methods;
/// bypassing them through SurfaceHost would desynchronize native-window guards.
/// Reparenting can recreate the XID and is allowed only while Detached.
/// A release-query error pauses automatic polling without discarding the
/// observer; call pollDetach() explicitly to retry after handling the error.
/// The complete public API is GUI-thread confined. Foreign control calls return
/// MIGO_ERROR_WRONG_THREAD without touching Qt state or entering Migo.
class MigoQtX11SurfaceView final : public QWidget {
    Q_OBJECT

public:
    MigoQtX11SurfaceView(SurfaceHost &surface_host, QWidget &parent);
    ~MigoQtX11SurfaceView() override;

    MigoQtX11SurfaceView(const MigoQtX11SurfaceView &) = delete;
    MigoQtX11SurfaceView &operator=(const MigoQtX11SurfaceView &) = delete;

    [[nodiscard]] MigoResult attachSurface();
    [[nodiscard]] MigoResult beginDetach();
    [[nodiscard]] MigoResult pollDetach(bool *released);

    [[nodiscard]] SurfaceState surfaceState() const noexcept {
        return owns_surface_ ? surface_host_.state() : SurfaceState::Detached;
    }
    [[nodiscard]] std::uint64_t generation() const noexcept { return owned_generation_; }
    [[nodiscard]] MigoResult lastError() const noexcept { return last_error_; }
    [[nodiscard]] bool isReleasePolling() const noexcept { return release_timer_.isActive(); }

Q_SIGNALS:
    void surfaceAttached(quint64 generation);
    void surfaceReleased(quint64 generation);
    void surfaceReleaseStalled(quint64 generation, qint64 elapsed_ms);
    void surfaceError(MigoResult error);

protected:
    [[nodiscard]] QPaintEngine *paintEngine() const override;
    void showEvent(QShowEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void closeEvent(QCloseEvent *event) override;
    bool event(QEvent *event) override;

private:
    [[nodiscard]] MigoResult currentMetrics(SurfaceMetrics *metrics) const noexcept;
    void scheduleMetricsUpdate();
    void updateReleasePollCadence();
    void recordError(MigoResult error);

    SurfaceHost &surface_host_;
    QThread *const owner_thread_;
    QTimer release_timer_;
    QTimer metrics_update_timer_;
    QElapsedTimer release_elapsed_;
    MigoResult last_error_ = MIGO_OK;
    std::uint64_t owned_generation_ = 0;
    bool owns_surface_ = false;
    bool close_pending_ = false;
    bool release_stall_reported_ = false;
    QMetaObject::Connection screen_changed_connection_;
};

}  // namespace migo::linux_host::qt6

#endif  // MIGO_LINUX_QT6_X11_SURFACE_VIEW_HPP_
