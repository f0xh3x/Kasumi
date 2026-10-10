# Sends a Windows notification (as PowerShell), to test StatusNotch's notification
# widget and pop-ups. Windows keeps it only while its notifications are on
# (Do not disturb is fine: it just doesn't show its own pop-up then).
#   powershell -File tools/toast.ps1 [-title "Mia"] [-text "Lunch?"]
param([string]$title = "StatusNotch test", [string]$text = "A notification to try the notch with.")
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null
$xml = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02)
$lines = $xml.GetElementsByTagName("text")
$lines.Item(0).AppendChild($xml.CreateTextNode($title)) > $null
$lines.Item(1).AppendChild($xml.CreateTextNode($text)) > $null
# PowerShell's own AppUserModelID: lets an unpackaged script send notifications.
$app = "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe"
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show([Windows.UI.Notifications.ToastNotification]::new($xml))
"sent: $title"
