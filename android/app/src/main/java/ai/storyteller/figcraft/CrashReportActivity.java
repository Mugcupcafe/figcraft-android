package ai.storyteller.figcraft;

import android.app.Activity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.graphics.Color;
import android.graphics.Typeface;
import android.os.Bundle;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

/**
 * Minimal pure-Java screen shown in an isolated :crash process when FigCraft fails on launch.
 * Lets the user copy the exact error log with one tap—no PC or ADB required.
 */
public class CrashReportActivity extends Activity {
    public static final String EXTRA_REPORT = "report";
    public static final String EXTRA_SAVED_PATH = "saved_path";

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        String report = getIntent().getStringExtra(EXTRA_REPORT);
        if (report == null) report = "(No crash details captured)";
        String savedPath = getIntent().getStringExtra(EXTRA_SAVED_PATH);
        if (savedPath == null) savedPath = "Downloads";

        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setBackgroundColor(Color.parseColor("#18181B"));
        int pad = (int) (20 * getResources().getDisplayMetrics().density);
        root.setPadding(pad, pad * 2, pad, pad);

        TextView title = new TextView(this);
        title.setText("FigCraft 실행 오류 로그");
        title.setTextColor(Color.parseColor("#F87171"));
        title.setTextSize(20);
        title.setTypeface(Typeface.DEFAULT_BOLD);
        root.addView(title);

        TextView sub = new TextView(this);
        sub.setText("저장 위치: " + savedPath);
        sub.setTextColor(Color.parseColor("#A1A1AA"));
        sub.setTextSize(13);
        sub.setPadding(0, pad / 3, 0, pad / 2);
        root.addView(sub);

        LinearLayout btnRow = new LinearLayout(this);
        btnRow.setOrientation(LinearLayout.HORIZONTAL);

        final String finalReport = report;
        Button copyBtn = new Button(this);
        copyBtn.setText("로그 전체 복사");
        copyBtn.setOnClickListener(v -> {
            ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
            if (cm != null) {
                cm.setPrimaryClip(ClipData.newPlainText("FigCraft Crash Log", finalReport));
                Toast.makeText(this, "클립보드에 복사되었습니다.", Toast.LENGTH_SHORT).show();
            }
        });
        btnRow.addView(copyBtn);

        Button closeBtn = new Button(this);
        closeBtn.setText("닫기");
        closeBtn.setOnClickListener(v -> finishAffinity());
        btnRow.addView(closeBtn);

        root.addView(btnRow);

        ScrollView scroll = new ScrollView(this);
        LinearLayout.LayoutParams sp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                0,
                1.0f
        );
        sp.topMargin = pad / 2;
        scroll.setLayoutParams(sp);

        TextView logView = new TextView(this);
        logView.setText(report);
        logView.setTextColor(Color.parseColor("#E4E4E7"));
        logView.setTextSize(12);
        logView.setTypeface(Typeface.MONOSPACE);
        logView.setTextIsSelectable(true);
        scroll.addView(logView);

        root.addView(scroll);
        setContentView(root);
    }
}
