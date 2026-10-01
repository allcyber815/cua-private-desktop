using System;
using System.Runtime.InteropServices;
using System.Windows.Forms;

namespace WebGPT.XamlIslandFixture;

internal static class WindowsAppSdkHelper
{
    public static void EnableContentPreTranslateMessageInEventLoop()
    {
        Application.AddMessageFilter(new ContentPreTranslateMessageFilter());
    }

    private sealed class ContentPreTranslateMessageFilter : IMessageFilter
    {
        public bool PreFilterMessage(ref Message m)
        {
            MSG msg = new(m);
            return ContentPreTranslateMessage(ref msg);
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MSG
    {
        public MSG(Message other)
        {
            hwnd = other.HWnd;
            message = (uint)other.Msg;
            wParam = other.WParam;
            lParam = other.LParam;
            time = 0;
            pt_x = 0;
            pt_y = 0;
        }
        public IntPtr hwnd;
        public uint message;
        public IntPtr wParam;
        public IntPtr lParam;
        public int time;
        public int pt_x;
        public int pt_y;
    }

    [DllImport("Microsoft.UI.Windowing.Core.dll")]
    private static extern bool ContentPreTranslateMessage(ref MSG message);
}
