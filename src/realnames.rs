//! Real gear names: the amp, pedal or cabinet each effect is modelled on.
//! The pedal's own names stay the default; a setting switches what is shown.
//! Names belong to their owners and are used only to describe the sound.
//! Cross-checked against the "based on" facts listed by hxtra's real_names
//! (wisemandenny/hxtra); written here in our own words.

use std::sync::atomic::{AtomicBool, Ordering};

static REAL: AtomicBool = AtomicBool::new(false);

pub fn set(on: bool) {
    REAL.store(on, Ordering::Relaxed);
}

pub fn on() -> bool {
    REAL.load(Ordering::Relaxed)
}

fn lookup(key: &str) -> Option<&'static str> {
    TABLE.iter().find(|(n, _)| n.eq_ignore_ascii_case(key)).map(|(_, r)| *r)
}

/// Channel endings the pedal adds to amp names, and how to say them.
const CHANNELS: &[(&str, &str)] = &[
    (" Nrm", "Normal"),
    (" Norm", "Normal"),
    (" Brt", "Bright"),
    (" Jump", "Jumped"),
    (" Vib", "Vibrato"),
    (" Ch1", "Channel 1"),
    (" Ch2", "Channel 2"),
    (" Ch 1", "Channel 1"),
    (" Ch 2", "Channel 2"),
    (" Clean", "Clean"),
    (" Crunch", "Crunch"),
    (" Lead", "Lead"),
    (" OD", "Overdrive"),
    (" Blue", "Blue"),
    (" Red", "Red"),
    (" Purple", "Purple"),
    (" Dirty", "Dirty"),
    (" Mega", "Mega"),
    (" Rhythm 1", "Rhythm 1"),
    (" Rhythm 2", "Rhythm 2"),
];

/// The real gear an effect is based on, when known.
pub fn real(name: &str) -> Option<String> {
    let key = name.trim();
    if let Some(r) = lookup(key) {
        return Some(r.to_owned());
    }
    for (end, said) in CHANNELS {
        if let Some(base) = key.strip_suffix(end) {
            if let Some(r) = lookup(base.trim()) {
                return Some(format!("{r} ({said})"));
            }
        }
    }
    None
}

/// The name to show for an effect, following the setting.
pub fn show(name: &str) -> String {
    if on() {
        if let Some(r) = real(name) {
            return r;
        }
    }
    name.to_owned()
}

/// The other name, for a hover hint ("Based on ..." or the pedal's name).
pub fn other(name: &str) -> Option<String> {
    let r = real(name)?;
    Some(if on() { format!("On the pedal: {name}") } else { format!("Based on {r}") })
}

const TABLE: &[(&str, &str)] = &[
    // Drives, distortions and fuzzes.
    ("Kinky Boost", "Xotic EP Booster"),
    ("Deranged Master", "Dallas Rangemaster Treble Booster"),
    ("Minotaur", "Klon Centaur"),
    ("Teemah!", "Paul Cochrane Timmy"),
    ("Heir Apparent", "Analogman Prince of Tone"),
    ("Tone Sovereign", "Analogman King of Tone"),
    ("Alpaca Rouge", "Way Huge Red Llama (modded)"),
    ("Compulsive Drive", "Fulltone OCD"),
    ("Dhyana Drive", "Hermida Zendrive"),
    ("Horizon Drive", "Horizon Devices Precision Drive"),
    ("Valve Driver", "Chandler Tube Driver"),
    ("Top Secret OD", "DOD OD-250"),
    ("Prize Drive", "Nobels ODR-1"),
    ("Scream 808", "Ibanez TS808 Tube Screamer"),
    ("Hedgehog D9", "Maxon SD9 Sonic Distortion"),
    ("Stupor OD", "Boss SD-1 Super Overdrive"),
    ("Deez One Vintage", "Boss DS-1 (Japan)"),
    ("Deez One Mod", "Boss DS-1 (modded)"),
    ("Ratatouille Dist", "ProCo RAT ('84)"),
    ("Vermin Dist", "ProCo RAT (vintage)"),
    ("KWB", "Ben Adrian KWB"),
    ("Pillars", "EarthQuaker Devices Plumes"),
    ("Vital Dist", "EarthQuaker Devices Life Pedal (distortion)"),
    ("Vital Boost", "EarthQuaker Devices Life Pedal (boost)"),
    ("Dark Dove Fuzz", "Electro-Harmonix Russian Big Muff"),
    ("Regal Bass DI", "Noble Preamp DI"),
    ("Killer Z", "Boss MT-2 Metal Zone"),
    ("Legendary Drive", "Carvin VLD1 Legacy Drive"),
    ("Swedish Chainsaw", "Boss HM-2 Heavy Metal"),
    ("Arbitrator Fuzz", "Arbiter Fuzz Face"),
    ("Pocket Fuzz", "Jordan Boss Tone"),
    ("Bighorn Fuzz", "Electro-Harmonix '73 Ram's Head Big Muff Pi"),
    ("Triangle Fuzz", "Electro-Harmonix Triangle Big Muff Pi"),
    ("Ballistic Fuzz", "Euthymia ICBM Fuzz"),
    ("Industrial Fuzz", "Z.Vex Fuzz Factory"),
    ("Tycoctavia Fuzz", "Tycobrahe Octavia"),
    ("Wringer Fuzz", "Boss FZ-2 (Garbage mod)"),
    ("Xenomorph Fuzz", "Subdecay Harmonic Antagonizer"),
    ("Ampeg Scrambler", "Ampeg Scrambler Bass Overdrive"),
    ("ZeroAmp Bass DI", "Tech 21 SansAmp Bass Driver DI"),
    ("Obsidian 7000", "Darkglass Microtubes B7K Ultra"),
    ("Clawthorn Drive", "Wounded Paw Battering Ram"),
    ("Tube Drive", "Chandler Tube Driver"),
    ("Screamer", "Ibanez TS-9 Tube Screamer"),
    ("Overdrive", "DOD 250 Overdrive/Preamp"),
    ("Classic Dist", "ProCo RAT"),
    ("Heavy Dist", "Boss Metal Zone"),
    ("Colordrive", "Colorsound Overdriver"),
    ("Buzz Saw", "Maestro Fuzz-Tone"),
    ("Facial Fuzz", "Arbiter Fuzz Face"),
    ("Jumbo Fuzz", "Vox Tone Bender"),
    ("Fuzz Pi", "Electro-Harmonix Big Muff Pi"),
    ("Jet Fuzz", "Roland Jet Phaser"),
    ("Sub Oct Fuzz", "PAiA Roctave Divider"),
    ("Octave Fuzz", "Tycobrahe Octavia"),
    ("Bronze Master", "Maestro Bass Brassmaster"),
    // Dynamics and EQ.
    ("Red Squeeze", "MXR Dyna Comp"),
    ("Kinky Comp", "Xotic SP Compressor"),
    ("Ampeg Opto Comp", "Ampeg Opto Comp"),
    ("Rochester Comp", "Ashly CLX-52"),
    ("LA Studio Comp", "Teletronix LA-2A"),
    ("Tube Comp", "Teletronix LA-2A"),
    ("Red Comp", "MXR Dyna Comp"),
    ("Blue Comp", "Boss CS-1 Compression Sustainer"),
    ("Blue Comp Treb", "Boss CS-1 (treble)"),
    ("Cali Q Graphic", "MESA/Boogie Mark IV graphic EQ"),
    // Modulation.
    ("Optical Trem", "Fender optical tremolo"),
    ("60s Bias Trem", "Vox AC15 tremolo"),
    ("Bleat Chop Trem", "Lightfoot Labs Goatkeeper"),
    ("Script Mod Phase", "MXR Phase 90 (script logo)"),
    ("Pebble Phaser", "Electro-Harmonix Small Stone"),
    ("Ubiquitous Vibe", "Shin-ei Uni-Vibe"),
    ("Gray Flanger", "MXR 117 Flanger"),
    ("Harmonic Flanger", "A/DA Flanger"),
    ("Courtesan Flange", "Electro-Harmonix Deluxe Electric Mistress"),
    ("70s Chorus", "Boss CE-1 Chorus Ensemble"),
    ("PlastiChorus", "Arion SCH-Z Chorus"),
    ("Ampeg Liquifier", "Ampeg Liquifier"),
    ("Trinity Chorus", "Dytronics Tri-Stereo Chorus"),
    ("Bubble Vibrato", "Boss VB-2 Vibrato"),
    ("Vibe Rotary", "Fender Vibratone"),
    ("122 Rotary", "Leslie 122"),
    ("145 Rotary", "Leslie 145"),
    ("Bias Tremolo", "Vox AC15 tremolo"),
    ("Tremolo", "Boss PN-2 Tremolo/Pan"),
    ("Tremolo/Autopan", "Boss PN-2 Tremolo/Pan"),
    ("Pitch Vibrato", "Boss VB-2 Vibrato"),
    ("Triple Rotary", "Roland RA-200"),
    ("Opto Tremolo", "Fender '64 Deluxe tremolo"),
    ("Script Phase", "MXR Phase 90 (script logo)"),
    ("Panned Phaser", "Ibanez Flying Pan"),
    ("Dual Phaser", "Mu-Tron Bi-Phase"),
    ("U-Vibe", "Shin-ei Uni-Vibe"),
    ("Phaser", "MXR Phase 90"),
    ("Dimension", "Roland Dimension D"),
    ("Analog Chorus", "Boss CE-1 Chorus Ensemble"),
    ("Tri Chorus", "Dytronics Tri-Stereo Chorus"),
    ("Analog Flanger", "MXR Flanger"),
    ("Jet Flanger", "A/DA Flanger"),
    ("AC Flanger", "MXR Flanger"),
    ("80A Flanger", "A/DA Flanger"),
    ("Rotary Drum", "Fender Vibratone"),
    ("Rotary Drum/Horn", "Leslie 145"),
    // Delays and reverbs.
    ("Ducked Delay", "TC Electronic 2290"),
    ("Transistor Tape", "Maestro Echoplex EP-3"),
    ("Cosmos Echo", "Roland RE-201 Space Echo"),
    ("Bucket Brigade", "Boss DM-2"),
    ("Adriatic Delay", "Boss DM-2 (Adrian mod)"),
    ("Adriatic Swell", "Boss DM-2 (Adrian mod) with swell"),
    ("Elephant Man", "Electro-Harmonix Deluxe Memory Man"),
    ("Tube Echo", "Maestro Echoplex EP-1"),
    ("Tape Echo", "Maestro Echoplex EP-3"),
    ("Echo Platter", "Binson Echorec"),
    ("Analog Echo", "Boss DM-2"),
    ("Analog w/Mod", "Electro-Harmonix Deluxe Memory Man"),
    ("Multi-Head", "Roland RE-101 Space Echo"),
    ("'63 Spring", "Fender '63 spring reverb"),
    // Pitch and filters.
    ("Pitch Wham", "DigiTech Whammy"),
    ("Twin Harmony", "Eventide H3000"),
    ("Asheville Pattrn", "Moog MF-105M MuRF"),
    ("10 Band Graphic", "MXR 10-Band EQ"),
    ("Acoustic Sim", "Boss AC-2 Acoustic Simulator"),
    ("Synth String", "Roland GR-700"),
    ("Attack Synth", "Korg X-911"),
    ("Boctaver", "Boss OC-2 Octave"),
    ("Bass Octaver", "EBS OctaBass"),
    ("Mutant Filter", "Musitronics Mu-Tron III"),
    ("Mystery Filter", "Korg A3"),
    ("V Tron", "Musitronics Mu-Tron III"),
    ("Tron Up", "Musitronics Mu-Tron III (up)"),
    ("Tron Down", "Musitronics Mu-Tron III (down)"),
    ("Seeker", "Z.Vex Seek Wah"),
    ("Obi Wah", "Oberheim VCF"),
    // Wahs.
    ("UK Wah 846", "Vox V846"),
    ("Teardrop 310", "Dunlop Cry Baby 310"),
    ("Teardrop Bass Q", "Dunlop 105Q Bass Wah"),
    ("Fassel", "Dunlop Cry Baby Super"),
    ("Weeper", "Arbiter Cry Baby"),
    ("Chrome", "Vox V847"),
    ("Chrome Custom", "Vox V847 (modded)"),
    ("Throaty", "RMC Real McCoy 1"),
    ("Colorful", "Colorsound Wah-fuzz"),
    ("Conductor", "Maestro Boomerang"),
    // Guitar amps (channel endings are added on their own).
    ("WhoWatt 100", "Hiwatt DR-103"),
    ("Soup Pro", "Supro S6616"),
    ("Stone Age 185", "Gibson EH-185"),
    ("Voltage Queen", "Victoria Electro King"),
    ("Tweed Blues", "Fender Bassman (tweed)"),
    ("Fullerton", "Fender 5C3 Tweed Deluxe"),
    ("GrammaticoLG", "Grammatico LaGrange"),
    ("US Small Tweed", "Fender Champ"),
    ("US Princess", "Fender Princeton Reverb"),
    ("US Super", "Fender Super Reverb"),
    ("US Deluxe", "Fender Deluxe Reverb"),
    ("US Double", "Fender Twin Reverb"),
    ("Mail Order Twin", "Silvertone 1484"),
    ("Divided Duo", "Divided by 13 JRT 9/15"),
    ("Interstate Zed", "Dr. Z Route 66"),
    ("Derailed Ingrid", "Trainwreck Express"),
    ("Grammatico GSG", "Grammatico GSG100"),
    ("Jazz Rivet 120", "Roland JC-120 Jazz Chorus"),
    ("Essex A15", "Vox AC15"),
    ("Essex A30", "Vox AC30 Top Boost"),
    ("A30 Fawn", "Vox AC30 Fawn"),
    ("Matchstick", "Matchless DC-30"),
    ("Matchstick Ch1", "Matchless DC-30 (Channel 1)"),
    ("Matchstick Ch2", "Matchless DC-30 (Channel 2)"),
    ("Mandarin 80", "Orange OR80"),
    ("Mandarin Rocker", "Orange Rockerverb 100 MkIII"),
    ("Moo)))n", "Sunn Model T"),
    ("Brit J45", "Marshall JTM45"),
    ("Brit Trem", "Marshall Plexi Tremolo"),
    ("Brit Plexi", "Marshall Super Lead 100"),
    ("Brit P75", "Park 75"),
    ("Brit 2203", "Marshall JCM800 2203"),
    ("Brit 2204", "Marshall JCM800 2204"),
    ("Placater", "Friedman BE-100"),
    ("Cartographer", "Ben Adrian Cartographer"),
    ("German Xtra", "Bogner Ecstasy"),
    ("German Mahadeva", "Bogner Shiva"),
    ("German Ubersonic", "Bogner Überschall"),
    ("Cali Texas", "MESA/Boogie Lone Star"),
    ("Cali IV", "MESA/Boogie Mark IV"),
    ("Cali Rectifire", "MESA/Boogie Dual Rectifier"),
    ("Archetype", "PRS Archon"),
    ("ANGL Meteor", "ENGL Fireball 100"),
    ("Solo Lead", "Soldano SLO-100"),
    ("EV Panama", "EVH 5150III"),
    ("PV Panama", "Peavey 5150"),
    ("PV Vitriol", "Peavey Invective"),
    ("PV VitriolCrunch", "Peavey Invective (Crunch)"),
    ("Revv Gen", "Revv Generator 120"),
    ("Das Benzin", "Diezel VH4"),
    // Bass amps.
    ("Ampeg B-15NF", "Ampeg B-15NF Portaflex"),
    ("Ampeg SVT", "Ampeg SVT"),
    ("Ampeg SVT-4 PRO", "Ampeg SVT-4 PRO"),
    ("Woody Blue", "Acoustic 360"),
    ("Agua Sledge", "Aguilar Tone Hammer"),
    ("Agua 51", "Aguilar DB751"),
    ("US Dripman", "Fender Silverface Bassman"),
    ("Mandarin Bass200", "Orange AD200"),
    ("Cali Bass", "MESA/Boogie M9 Carbine"),
    ("Cali 400", "MESA/Boogie Bass 400+"),
    ("G Cougar 800", "Gallien-Krueger GK 800RB"),
    ("Del Sol 300", "Sunn Coliseum 300"),
    ("Busy One", "Pearce BC-1"),
    // Cabinets.
    ("Soup Pro Ellipse", "Supro S6616 6x9"),
    ("1x8 Small Tweed", "Fender Champ 1x8"),
    ("1x10 US Princess", "Fender Princeton 1x10"),
    ("1x12 Fullerton", "Fender 5C3 Tweed Deluxe 1x12"),
    ("1x12 Grammatico", "Grammatico LaGrange 1x12"),
    ("1x12 US Deluxe", "Fender Deluxe Reverb 1x12"),
    ("1x12 US Princess", "Fender Princeton Reverb 1x12"),
    ("1x12 Field Coil", "Gibson EH-185 1x12 field coil"),
    ("1x12 Cali EXT", "MESA/Boogie 1x12 extension"),
    ("1x12 Cali IV", "MESA/Boogie Mark IV 1x12"),
    ("1x12 Blue Bell", "Vox AC15 1x12 Celestion Blue"),
    ("2x12 Blue Bell", "Vox AC30 2x12 Celestion Blue"),
    ("2x12 Silver Bell", "Vox AC30TB 2x12 Silver Bell"),
    ("2x12 Match H30", "Matchless DC-30 2x12 G12H30"),
    ("2x12 Match G25", "Matchless DC-30 2x12 G12M-25"),
    ("2x12 Double C12N", "Fender Twin Reverb 2x12 Jensen C12N"),
    ("2x12 Interstate", "Dr. Z Z Best 2x12"),
    ("2x12 Jazz Rivet", "Roland JC-120 2x12"),
    ("2x12 Mail C12Q", "Silvertone 1484 2x12 Jensen C12Q"),
    ("2x12 Mandarin 30", "Orange 2x12 Vintage 30"),
    ("4x10 Tweed P10R", "Fender Bassman 4x10 Jensen P10R"),
    ("4x10 US Super", "Fender Super Reverb 4x10"),
    ("4x12 WhoWatt 100", "Hiwatt 4x12 Fane"),
    ("4x12 Greenback 20", "Marshall 4x12 Celestion G12M-20"),
    ("4x12 Greenback 25", "Marshall 4x12 Celestion G12M-25"),
    ("4x12 Greenback 30", "Marshall 4x12 Celestion G12H-30"),
    ("4x12 1960A T75", "Marshall 1960A 4x12 G12T-75"),
    ("4x12 1960 T75", "Marshall 1960 4x12 G12T-75"),
    ("4x12 Brit V30", "Marshall 1960AV 4x12 Vintage 30"),
    ("4x12 MOO)))N T75", "Sunn 4x12 G12T-75"),
    ("4x12 Blackback 30", "Park 4x12 G12H-30"),
    ("2x15 US Dripman", "Fender Bassman 2x15"),
    ("2x15 Brute", "MESA/Boogie 2x15 EV"),
    ("4x10 Garden", "Eden D410 4x10"),
    ("1x12 Epicenter", "Epifani 1x12"),
    ("1x12 Celest 12H", "Divided by 13 1x12 G12H-30"),
    ("1x12 Lead 80", "Bogner Shiva 1x12 CL80"),
    ("4x12 Cali V30", "MESA/Boogie 4x12 Vintage 30"),
    ("4x12 Mandarin EM", "Orange 4x12"),
    ("4x12 Cartog Guv", "Ben Adrian 4x12 (Guv'nor speakers)"),
    ("4x12 Uber T75", "Bogner Uberkab 4x12 G12T-75"),
    ("4x12 Uber V30", "Bogner Uberkab 4x12 Vintage 30"),
    ("4x12 XXL V30", "ENGL XXL 4x12 Vintage 30"),
    ("4x12 SoloLead EM", "Soldano 4x12"),
    ("1x15 Ampeg B-15", "Ampeg B-15 1x15"),
    ("4x10 Ampeg Pro", "Ampeg PR-410HLF 4x10"),
    ("4x10 Ampeg HLF", "Ampeg SVT-410HLF 4x10"),
    ("6x10 Cali Power", "MESA/Boogie Power House 6x10"),
    ("8x10 SVT AV", "Ampeg SVT-810AV 8x10"),
    ("8x10 Ampeg SVT E", "Ampeg SVT-810E 8x10"),
    ("1x18 Woody Blue", "Acoustic 360 1x18"),
    ("1x18 Del Sol", "Sunn Coliseum 1x18"),
    ("1x12 Del Sol", "Sunn Coliseum 1x12"),
];
