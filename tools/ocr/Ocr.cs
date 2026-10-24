using System;
using System.IO;
using Windows.Graphics.Imaging;
using Windows.Media.Ocr;
using Windows.Globalization;
using Windows.Storage.Streams;

class Ocr {
  static void Main(string[] args) {
    byte[] bytes = File.ReadAllBytes(args[0]);
    using (var stream = new InMemoryRandomAccessStream()) {
      using (var writer = new DataWriter(stream)) {
        writer.WriteBytes(bytes);
        writer.StoreAsync().AsTask().Wait();
        writer.DetachStream();
      }
      stream.Seek(0);
      var decoderTask = BitmapDecoder.CreateAsync(stream).AsTask();
      decoderTask.Wait();
      var decoder = decoderTask.Result;
      var bmpTask = decoder.GetSoftwareBitmapAsync().AsTask();
      bmpTask.Wait();
      var bmp = bmpTask.Result;
      var engine = OcrEngine.TryCreateFromLanguage(new Language("en-US"));
      if (engine == null) { Console.WriteLine("no engine"); return; }
      var ocrTask = engine.RecognizeAsync(bmp).AsTask();
      ocrTask.Wait();
      var result = ocrTask.Result;
      Console.WriteLine("TEXT_START");
      Console.WriteLine(result.Text);
      Console.WriteLine("TEXT_END");
      Console.WriteLine("LINES: " + result.Lines.Count);
      foreach (var line in result.Lines) {
        Console.WriteLine("LINE: " + line.Text);
        foreach (var w in line.Words) {
            Console.WriteLine(string.Format("WORD[{0},{1},{2},{3}]: {4}", (int)w.BoundingRect.Left, (int)w.BoundingRect.Top, (int)w.BoundingRect.Width, (int)w.BoundingRect.Height, w.Text));
        }
      }
    }
  }
}
