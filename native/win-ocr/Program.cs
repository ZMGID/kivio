using System;
using System.IO;
using System.Threading.Tasks;
using Windows.Graphics.Imaging;
using Windows.Media.Ocr;
using Windows.Storage;

namespace KeyLingo.Ocr
{
    class Program
    {
        static async Task Main(string[] args)
        {
            if (args.Length < 1)
            {
                Console.Error.WriteLine("Usage: KeyLingo.Ocr.exe <ImagePath>");
                Environment.Exit(1);
            }

            string imagePath = args[0];

            if (!File.Exists(imagePath))
            {
                Console.Error.WriteLine($"Error: File not found: {imagePath}");
                Environment.Exit(1);
            }

            try
            {
                string result = await RecognizeTextAsync(imagePath);
                Console.Out.Write(result); // Use Write to avoid trailing newline if possible
            }
            catch (Exception ex)
            {
                Console.Error.WriteLine($"Error: {ex.Message}");
                Environment.Exit(1);
            }
        }

        static async Task<string> RecognizeTextAsync(string imagePath)
        {
            // Load file
            StorageFile file = await StorageFile.GetFileFromPathAsync(Path.GetFullPath(imagePath));
            using var stream = await file.OpenAsync(FileAccessMode.Read);
            
            // Decode image
            BitmapDecoder decoder = await BitmapDecoder.CreateAsync(stream);
            using var softwareBitmap = await decoder.GetSoftwareBitmapAsync();

            // Init OCR engine
            OcrEngine engine = OcrEngine.TryCreateFromUserProfileLanguages();
            if (engine == null)
            {
                throw new Exception("OCR engine could not be initialized. Please check Windows language pack installation.");
            }

            // Recognize
            OcrResult result = await engine.RecognizeAsync(softwareBitmap);

            return result.Text; // Returns full text associated with the result
        }
    }
}
