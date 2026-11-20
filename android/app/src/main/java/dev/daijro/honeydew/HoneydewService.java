package dev.daijro.honeydew;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.os.Process;

// Keeps the app running in the background while the song plays or an export renders, as FL Studio Mobile does:
// Android stops or freezes a background app that has no foreground service. HoneydewActivity.onPause starts it when
// there is something to keep doing, onResume stops it. Its notification brings the app back.
public final class HoneydewService extends Service {
    private static final String CHANNEL = "background";
    private static final int ID = 1;

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel(
                CHANNEL, "Playing in the background", NotificationManager.IMPORTANCE_LOW));
        Intent open = getPackageManager().getLaunchIntentForPackage(getPackageName());
        PendingIntent tap = open == null ? null : PendingIntent.getActivity(
                this, 0, open, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification notification = new Notification.Builder(this, CHANNEL)
                .setSmallIcon(getApplicationInfo().icon)
                .setContentTitle(getPackageManager().getApplicationLabel(getApplicationInfo()))
                .setContentText("Running in the background")
                .setContentIntent(tap)
                .setOngoing(true)
                .build();
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK);
        } else {
            startForeground(ID, notification);
        }
        return START_NOT_STICKY;
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    // Swiped away from the recent apps: the app closes, as it did without the service (its changes were kept when it
    // left the screen).
    @Override
    public void onTaskRemoved(Intent rootIntent) {
        stopSelf();
        Process.killProcess(Process.myPid());
    }
}
