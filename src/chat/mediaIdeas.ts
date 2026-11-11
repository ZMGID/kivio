import type { MediaKind } from '../api/mediaStation'
import type { IdeaArtName } from './MediaIdeaArt'

export type Idea = { art: IdeaArtName; ratio: string; title: readonly [string, string]; prompt: readonly [string, string] }

/** Starter prompts per media kind, each with its own line drawing and a fitting aspect ratio. */
export const IDEAS: Record<MediaKind, readonly Idea[]> = {
  image: [
    { art: 'vase', ratio: '1:1', title: ['产品摄影', 'Product photo'], prompt: ['晨光中的玻璃花瓶，简洁背景，柔和阴影，产品摄影。', 'A glass vase in morning light, minimal background, soft shadows, product photography.'] },
    { art: 'coast', ratio: '16:9', title: ['电影感场景', 'Cinematic scene'], prompt: ['夕阳下的海岸，暖金色光线，宽阔构图，电影质感。', 'A coastline at sunset, warm golden light, wide composition, cinematic atmosphere.'] },
    { art: 'poster', ratio: '3:4', title: ['插画海报', 'Illustrated poster'], prompt: ['以森林与月亮为主题的插画海报，深蓝与银白配色，留出标题空间。', 'An illustrated forest and moon poster in midnight blue and silver, with room for a title.'] },
    { art: 'portrait', ratio: '3:4', title: ['人像写真', 'Portrait'], prompt: ['窗边自然光下的人像特写，浅景深，胶片质感，温柔色调。', 'A close-up portrait by a window in natural light, shallow depth of field, film look, soft tones.'] },
    { art: 'coffee', ratio: '1:1', title: ['美食摄影', 'Food photo'], prompt: ['俯拍木桌上的拿铁，心形拉花，晨光斜照，温暖色调。', 'A top-down latte with heart latte art on a wooden table, slanted morning light, warm tones.'] },
    { art: 'arches', ratio: '4:3', title: ['建筑光影', 'Architecture'], prompt: ['极简混凝土拱廊，强烈的光影，一个人走过，建筑摄影。', 'A minimal concrete arcade with strong light and shadow, a lone figure walking through, architectural photography.'] },
    { art: 'room', ratio: '1:1', title: ['等距小屋', 'Isometric room'], prompt: ['等距视角的温馨小书房，柔和光照，3D 渲染。', 'A cozy isometric study room, soft lighting, 3D render.'] },
    { art: 'shanshui', ratio: '16:9', title: ['水墨山水', 'Ink landscape'], prompt: ['水墨山水，远山与云雾，一叶扁舟，大面积留白。', 'An ink-wash landscape with distant mountains, mist and a small boat, generous negative space.'] },
    { art: 'interior', ratio: '4:3', title: ['室内设计', 'Interior'], prompt: ['北欧风客厅，浅色沙发，落地灯，绿植，自然光，室内效果图。', 'A Scandinavian living room with a light sofa, floor lamp and plants in natural light, interior render.'] },
    { art: 'cat', ratio: '1:1', title: ['宠物写真', 'Pet portrait'], prompt: ['一只蜷在毛毯上睡觉的橘猫，午后阳光，柔焦。', 'An orange cat curled up asleep on a blanket in afternoon sun, soft focus.'] },
    { art: 'leaf', ratio: '3:4', title: ['植物图鉴', 'Botanical plate'], prompt: ['龟背竹叶片的植物图鉴，复古科学插画风格，米色纸张。', 'A botanical plate of a monstera leaf, vintage scientific illustration on cream paper.'] },
    { art: 'street', ratio: '9:16', title: ['雨夜街头', 'Rainy street'], prompt: ['雨夜的街头，霓虹倒映在积水里，撑伞的行人，赛博朋克。', 'A rainy street at night, neon reflected in puddles, a pedestrian with an umbrella, cyberpunk.'] },
  ],
  video: [
    { art: 'skyline', ratio: '16:9', title: ['航拍推进', 'Aerial push-in'], prompt: ['黄昏的城市天际线，无人机缓慢向前推进，灯光逐渐亮起。', 'A city skyline at dusk, a drone slowly pushes forward as the lights come on.'] },
    { art: 'turntable', ratio: '1:1', title: ['产品旋转', 'Product turntable'], prompt: ['白色背景上的运动鞋缓慢 360 度旋转，柔和棚拍光。', 'A sneaker slowly rotates 360 degrees on a white background in soft studio light.'] },
    { art: 'peaks', ratio: '16:9', title: ['自然延时', 'Nature time-lapse'], prompt: ['云海在雪山间流动的延时摄影，日出时分，金色光线。', 'A time-lapse of clouds flowing between snowy peaks at sunrise, golden light.'] },
    { art: 'fox', ratio: '16:9', title: ['动画角色', 'Animated character'], prompt: ['一只小狐狸在森林边蹦跳，手绘动画风格，镜头跟随。', 'A small fox hops at the edge of a forest in a hand-drawn animation style, the camera follows.'] },
    { art: 'waves', ratio: '16:9', title: ['海浪慢镜', 'Slow-mo wave'], prompt: ['巨浪卷起的慢动作，阳光穿透浪尖，水花飞溅。', 'A huge wave curling in slow motion, sunlight through the crest, spray flying.'] },
    { art: 'road', ratio: '16:9', title: ['公路追车', 'Road chase'], prompt: ['跑车在沙漠公路上疾驰，低机位跟拍，黄昏逆光。', 'A sports car speeds down a desert highway, low tracking shot, backlit at dusk.'] },
    { art: 'pour', ratio: '9:16', title: ['液体慢镜', 'Pour shot'], prompt: ['冰饮倒入玻璃杯的慢镜头，冰块翻滚，气泡上升。', 'Slow motion of a cold drink poured into a glass, ice tumbling, bubbles rising.'] },
    { art: 'blossom', ratio: '9:16', title: ['花开延时', 'Blooming'], prompt: ['一朵花从花苞到盛开的延时摄影，黑色背景，微距。', 'A time-lapse of a flower opening from bud to full bloom, black background, macro.'] },
    { art: 'walk', ratio: '9:16', title: ['人物跟拍', 'Tracking shot'], prompt: ['女孩走在秋天的林荫道上，侧面跟拍，落叶飘下。', 'A girl walks along an autumn avenue, side tracking shot, leaves falling.'] },
    { art: 'rainwin', ratio: '16:9', title: ['雨窗氛围', 'Rainy window'], prompt: ['雨滴顺着窗玻璃滑落，窗外城市灯光虚化，浅景深。', 'Raindrops run down a window, city lights blurred outside, shallow depth of field.'] },
    { art: 'fireworks', ratio: '16:9', title: ['烟花夜景', 'Fireworks'], prompt: ['夜空中绽放的烟花，城市剪影，镜头缓慢上摇。', 'Fireworks bursting over a city skyline at night, the camera slowly tilts up.'] },
    { art: 'plane', ratio: '16:9', title: ['纸飞机', 'Paper plane'], prompt: ['一架纸飞机穿过云层飞行，镜头跟随，绘本风格。', 'A paper plane glides through clouds, the camera follows, storybook style.'] },
  ],
}
export const IDEAS_PER_PAGE = 6

/** One prompt modifier; the inserted term follows the interface language. */
export type FacetTerm = readonly [string, string]
export type Facet = { label: readonly [string, string]; terms: readonly FacetTerm[] }

/** Modifiers layered onto a scene. Toggling one adds or removes exactly that term in the prompt. */
export const FACETS: Record<MediaKind, readonly Facet[]> = {
  image: [
    { label: ['风格', 'Style'], terms: [['写实摄影', 'photorealistic'], ['电影感', 'cinematic'], ['胶片', 'film grain'], ['插画', 'illustration'], ['水彩', 'watercolor'], ['3D 渲染', '3D render'], ['极简', 'minimalist']] },
    { label: ['光线', 'Light'], terms: [['晨光', 'morning light'], ['黄昏逆光', 'golden-hour backlight'], ['柔光棚拍', 'soft studio light'], ['霓虹', 'neon glow'], ['阴天漫射', 'overcast light']] },
    { label: ['构图', 'Framing'], terms: [['特写', 'close-up'], ['俯拍', 'top-down'], ['广角', 'wide angle'], ['对称构图', 'symmetrical'], ['大面积留白', 'negative space']] },
  ],
  video: [
    { label: ['镜头', 'Camera'], terms: [['缓慢推进', 'slow push-in'], ['侧面跟拍', 'side tracking shot'], ['环绕', 'orbit shot'], ['航拍', 'aerial'], ['手持', 'handheld']] },
    { label: ['节奏', 'Pace'], terms: [['慢动作', 'slow motion'], ['延时', 'time-lapse'], ['一镜到底', 'single take']] },
    { label: ['风格', 'Style'], terms: [['电影感', 'cinematic'], ['纪录片', 'documentary'], ['手绘动画', 'hand-drawn animation'], ['胶片', 'film grain']] },
  ],
}
