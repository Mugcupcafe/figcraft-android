package ai.storyteller.figcraft;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;

import androidx.core.app.NotificationCompat;

/**
 * Keeps the embedded FigCraft Streamable HTTP / SSE MCP server alive
 * while the user switches briefly to an AI client app on the Android tablet.
 *
 * V2 fix: Passes ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC on Android 14+ (API 34+)
 * to prevent MissingForegroundServiceTypeException crashes.
 */
public class McpForegroundService extends Service {
    public static final String EXTRA_ENDPOINT = "endpoint";
    public static final String ACTION_STOP = "ai.storyteller.figcraft.ACTION_STOP_MCP";
    private static final String CHANNEL_ID = "figcraft_mcp_channel";
    private static final int NOTIFICATION_ID = 47800;

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && ACTION_STOP.equals(intent.getAction())) {
            FigCraftActivity.notifyMcpStopped();
            stopForeground(STOP_FOREGROUND_REMOVE);
            stopSelf();
            return START_NOT_STICKY;
        }

        String endpoint = intent != null && intent.getStringExtra(EXTRA_ENDPOINT) != null
                ? intent.getStringExtra(EXTRA_ENDPOINT)
                : "http://127.0.0.1:47800/mcp";

        ensureChannel();

        Intent stopIntent = new Intent(this, McpForegroundService.class);
        stopIntent.setAction(ACTION_STOP);
        PendingIntent stopPending = PendingIntent.getService(
                this,
                0,
                stopIntent,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE
        );

        Notification notification = new NotificationCompat.Builder(this, CHANNEL_ID)
                .setContentTitle("FigCraft MCP Server Active")
                .setContentText("Listening on " + endpoint)
                .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
                .setOngoing(true)
                .addAction(android.R.drawable.ic_menu_close_clear_cancel, "Stop", stopPending)
                .build();

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC);
        } else {
            startForeground(NOTIFICATION_ID, notification);
        }
        return START_STICKY;
    }

    private void ensureChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationChannel channel = new NotificationChannel(
                    CHANNEL_ID,
                    "FigCraft MCP Server",
                    NotificationManager.IMPORTANCE_LOW
            );
            NotificationManager nm = getSystemService(NotificationManager.class);
            if (nm != null) {
                nm.createNotificationChannel(channel);
            }
        }
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
