
import { Jimp } from 'jimp';
import path from 'path';

async function processIcon() {
    const inputPath = path.resolve('public/icon.png');
    console.log(`Processing: ${inputPath}`);

    try {
        const image = await Jimp.read(inputPath);

        // macOS Big Sur+ icon shape is roughly a rounded rect with specialized curvature.
        // We'll use a standard rounded rect for approximation.
        // Resize to standard size if needed (e.g. 1024x1024)
        image.resize({ w: 512, h: 512 });

        // Draw a white rounded rectangle on the mask
        // Squircle approximation: r = 22% of size ideally, circular is simpler for jimp
        // Jimp doesn't have advanced drawing. 
        // Let's iterate pixels to mask corners.

        const width = 512;
        const height = 512;
        const radius = 110; // Approx 22% of 512

        // Process pixels
        for (let y = 0; y < height; y++) {
            for (let x = 0; x < width; x++) {
                // Check if (x,y) is outside the rounded corners
                // 4 corners:
                // Top-Left
                if (x < radius && y < radius) {
                    if (Math.hypot(x - radius, y - radius) > radius) {
                        image.setPixelColor(0x00000000, x, y);
                    }
                }
                // Top-Right
                else if (x > width - radius && y < radius) {
                    if (Math.hypot(x - (width - radius), y - radius) > radius) {
                        image.setPixelColor(0x00000000, x, y);
                    }
                }
                // Bottom-Left
                else if (x < radius && y > height - radius) {
                    if (Math.hypot(x - radius, y - (height - radius)) > radius) {
                        image.setPixelColor(0x00000000, x, y);
                    }
                }
                // Bottom-Right
                else if (x > width - radius && y > height - radius) {
                    if (Math.hypot(x - (width - radius), y - (height - radius)) > radius) {
                        image.setPixelColor(0x00000000, x, y);
                    }
                }
            }
        }

        await image.write('public/icon.png');
        console.log('Icon processed successfully.');

    } catch (err) {
        console.error('Error processing icon:', err);
    }
}

processIcon();
