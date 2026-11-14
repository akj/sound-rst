# The CC0 sound candidates, read by fetch-sound-candidates.ps1 and
# audition-sounds.ps1. docs/sound-sources.md explains each choice.
# Above4k is the percent of a file's energy above 4 kHz and Edge99kHz the
# frequency below which 99 percent of it lies, measured with
# scripts/measure-brightness.py on the downloaded file.
@{
    Candidates = @(
        @{
            Role = 'footsteps-grass'
            Folder = '1-giocosound'
            Kind = 'steps'
            Title = 'Footstep_Grass_1 to 5, five single steps'
            Page = 'https://freesound.org/people/GiocoSound/sounds/421131/ (the other steps are in the same pack, https://freesound.org/people/GiocoSound/packs/23825/)'
            Author = 'GiocoSound'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Five separate single steps on grass, mono, the brightest grass found.'
            Files = @(
                @{ Name = '421131.wav'; FreesoundId = 421131; Sha256 = '3543282B184809B5463A2CF2584356109AF60472AA4940064521A83E14444209'; Format = 'WAV'; SampleRate = 48000; Channels = 1; Seconds = 0.50; Above4k = 60.1; Edge99kHz = 19.6 }
                @{ Name = '421130.wav'; FreesoundId = 421130; Sha256 = '09F1EA23F35F93CEF2E7F86E399B36AEDCAC96EC8D09472B8EF0DCF8AE64E28E'; Format = 'WAV'; SampleRate = 48000; Channels = 1; Seconds = 0.50; Above4k = 62.9; Edge99kHz = 19.7 }
                @{ Name = '421129.wav'; FreesoundId = 421129; Sha256 = '2D293F6A1F75AD06C0F7F508563C7510855C5B4D4906634DB211E7D55021B721'; Format = 'WAV'; SampleRate = 48000; Channels = 1; Seconds = 0.50; Above4k = 56.5; Edge99kHz = 20.6 }
                @{ Name = '421128.wav'; FreesoundId = 421128; Sha256 = '1358092B5207EF9B57A8ACE21B5DA019CFD1C2D9C215DCF243098D38B55DAD82'; Format = 'WAV'; SampleRate = 48000; Channels = 1; Seconds = 0.50; Above4k = 58.1; Edge99kHz = 19.7 }
                @{ Name = '421135.wav'; FreesoundId = 421135; Sha256 = '97458E30CD239E9557A8EE1CF06D832D635758E2894A2B7ED99485F92652FC56'; Format = 'WAV'; SampleRate = 48000; Channels = 1; Seconds = 0.51; Above4k = 60.4; Edge99kHz = 20.2 }
            )
        }
        @{
            Role = 'footsteps-grass'
            Folder = '2-silentstrikez'
            Kind = 'walk'
            Title = 'Footsteps_Grass_1, a walk'
            Page = 'https://freesound.org/people/SilentStrikeZ/sounds/389625/'
            Author = 'SilentStrikeZ'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'A mono foley walk on grass, 19 seconds; would need cutting into steps.'
            Files = @(
                @{ Name = '389625.wav'; FreesoundId = 389625; Sha256 = 'E54D5C16A29C48164AF32A7A7274D9394AB0F2B334F524780065D618BE674742'; Format = 'WAV'; SampleRate = 48000; Channels = 1; Seconds = 19.41; Above4k = 25.9; Edge99kHz = 18.1 }
            )
        }
        @{
            Role = 'footsteps-grass'
            Folder = '3-nox-sound'
            Kind = 'walk'
            Title = 'Footsteps_Mountain_Boots_Grass_Mono, a walk in boots'
            Page = 'https://freesound.org/people/Nox_Sound/sounds/556042/'
            Author = 'Nox_Sound'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = '19 walking steps in hiking boots, then running and jumps; darker.'
            Files = @(
                @{ Name = '556042.wav'; FreesoundId = 556042; Sha256 = '2BFC1C7010AC0F6314CA653275DF97F5371C7C6AEA43F75B320B0CD324D405DE'; Format = 'WAV'; SampleRate = 48000; Channels = 1; Seconds = 69.30; Above4k = 14.6; Edge99kHz = 16.8 }
            )
        }
        @{
            Role = 'footsteps-leaves'
            Folder = '1-joao-janz'
            Kind = 'steps'
            Title = 'Sneaker Shoes Forest Footstep 1_1 to 1_5, five single steps'
            Page = 'https://freesound.org/people/Joao_Janz/sounds/504242/ (the other steps are in the same pack, https://freesound.org/people/Joao_Janz/packs/28240/)'
            Author = 'Joao_Janz'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Five single steps in trainers on a forest floor, de-noised by the author; the pack has about 15 more.'
            Files = @(
                @{ Name = '504242.wav'; FreesoundId = 504242; Sha256 = 'F766CADA66CEBF71B2D46B193EA87D3F005D9F888F4A82897B8AA05A5C0E5028'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 0.62; Above4k = 67.3; Edge99kHz = 16.6 }
                @{ Name = '504323.wav'; FreesoundId = 504323; Sha256 = '75836B9B1B4E03D8C7DD0E4C4FFC8FD5701CF9F7A75B7286EE08C3D0BB238212'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 0.62; Above4k = 91.6; Edge99kHz = 17.1 }
                @{ Name = '504324.wav'; FreesoundId = 504324; Sha256 = 'B0DD3667A185F8550EE77B139C6905110F82CB0000DE744F7C7AEA071510B56B'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 0.65; Above4k = 74.0; Edge99kHz = 15.5 }
                @{ Name = '504321.wav'; FreesoundId = 504321; Sha256 = '5CE081E76CE9085A2D4DA2BDF980BB1DC4D41CE00D7E765760179CBFDEC3EA3A'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 0.71; Above4k = 71.9; Edge99kHz = 16.4 }
                @{ Name = '504322.wav'; FreesoundId = 504322; Sha256 = '47672E8610B6F718DBDACDE92A4B55C5F86B1AD8C471C546525FA4D65950A86F'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 0.88; Above4k = 88.5; Edge99kHz = 16.9 }
            )
        }
        @{
            Role = 'footsteps-leaves'
            Folder = '2-anthousai'
            Kind = 'walk'
            Title = 'footsteps - leaves 01, a walk'
            Page = 'https://freesound.org/people/Anthousai/sounds/398776/'
            Author = 'Anthousai'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'A close walk through leaves, microphone near the feet, 96 kHz stereo.'
            Files = @(
                @{ Name = '398776.wav'; FreesoundId = 398776; Sha256 = '0AC3B3697A374C0C15857AB0003BD99059590A37D9E3A793A59D85625D91F3D7'; Format = 'WAV'; SampleRate = 96000; Channels = 2; Seconds = 17.78; Above4k = 69.3; Edge99kHz = 15.8 }
            )
        }
        @{
            Role = 'footsteps-leaves'
            Folder = '3-juno24'
            Kind = 'walk'
            Title = 'Footsteps forest, a few steps on pine needles'
            Page = 'https://freesound.org/people/Juno24/sounds/474113/'
            Author = 'Juno24'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Mono, crunchy dry pine needles rather than broad leaves.'
            Files = @(
                @{ Name = '474113.wav'; FreesoundId = 474113; Sha256 = 'F5195FA2363C47FDCD80F6F26C8C4C49C93612B98EF86619F957AB2D8C4EF8DA'; Format = 'WAV'; SampleRate = 48000; Channels = 1; Seconds = 23.41; Above4k = 25.4; Edge99kHz = 17.1 }
            )
        }
        @{
            Role = 'footsteps-gravel'
            Folder = '1-potatokingxii'
            Kind = 'walk'
            Title = 'Footsteps Dirt Gravel, a walk'
            Page = 'https://freesound.org/people/PotatokingXII/sounds/352870/'
            Author = 'PotatokingXII'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'A cleaned walk on dirt and gravel, shotgun microphone; only moderately bright.'
            Files = @(
                @{ Name = '352870.wav'; FreesoundId = 352870; Sha256 = '779A6DF49C983733A37D59DCA34BA86A6D8DF550A1AC5738D95243F98E1ECAEA'; Format = 'WAV'; SampleRate = 48000; Channels = 1; Seconds = 35.00; Above4k = 11.0; Edge99kHz = 12.2 }
            )
        }
        @{
            Role = 'footsteps-gravel'
            Folder = '2-ali-6868'
            Kind = 'steps'
            Title = 'Left and Right Gravel Footstep, six single steps'
            Page = 'https://freesound.org/people/Ali_6868/sounds/384873/ (the other steps are in the same pack, https://freesound.org/people/Ali_6868/packs/21608/)'
            Author = 'Ali_6868'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Six separate single steps on gravel; most are dark.'
            Files = @(
                @{ Name = '384873.wav'; FreesoundId = 384873; Sha256 = '7E7DB7FB5DA1EC7EBB60AA1645F0C03360443122797EB27286FE1B0DAD045EE1'; Format = 'WAV'; SampleRate = 44100; Channels = 2; Seconds = 0.69; Above4k = 4.8; Edge99kHz = 11.3 }
                @{ Name = '384871.wav'; FreesoundId = 384871; Sha256 = 'FC58B45E5C7FF1A03BB85DC21930EF4E4964D1DA6374B4D7CCB8246D1395BE93'; Format = 'WAV'; SampleRate = 44100; Channels = 2; Seconds = 0.69; Above4k = 4.9; Edge99kHz = 11.3 }
                @{ Name = '384877.wav'; FreesoundId = 384877; Sha256 = '120138B83AE7B9BCDDB8ED8F6B9A7109F4F1E5DC3E43DF535D3A82524B2F1939'; Format = 'WAV'; SampleRate = 44100; Channels = 2; Seconds = 0.48; Above4k = 3.5; Edge99kHz = 10.8 }
                @{ Name = '384876.wav'; FreesoundId = 384876; Sha256 = '3AB201DBC9846A82ED3435654B8A946032C48092F489894FC4A4A66494547614'; Format = 'WAV'; SampleRate = 44100; Channels = 2; Seconds = 0.48; Above4k = 5.2; Edge99kHz = 11.8 }
                @{ Name = '384879.wav'; FreesoundId = 384879; Sha256 = '3B72530E582B090C08C203C6CB7DB132D85EDA179BF50CFE921D0F8E07DA0BFF'; Format = 'WAV'; SampleRate = 44100; Channels = 2; Seconds = 0.48; Above4k = 10.7; Edge99kHz = 13.5 }
                @{ Name = '384880.wav'; FreesoundId = 384880; Sha256 = '36FC085BA5477BB03CCCEE01C0FBFF0FF629440423C251CAEEC4A32B47F9FD1F'; Format = 'WAV'; SampleRate = 44100; Channels = 2; Seconds = 0.57; Above4k = 16.1; Edge99kHz = 14.0 }
            )
        }
        @{
            Role = 'footsteps-wood'
            Folder = '1-kbeezy88'
            Kind = 'walk'
            Title = 'walking_on_boardwalk, a walk on a wooden boardwalk'
            Page = 'https://freesound.org/people/kbeezy88/sounds/494972/'
            Author = 'kbeezy88'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Footsteps on a wooden boardwalk outdoors; dark, like every wood step found, but brighter than the game''s own.'
            Files = @(
                @{ Name = '494972.wav'; FreesoundId = 494972; Sha256 = '3C56B3363209F6184B8C6632B657381E77C97D890FFC1F134A2D0A8055FDE114'; Format = 'WAV'; SampleRate = 44100; Channels = 2; Seconds = 76.80; Above4k = 3.6; Edge99kHz = 9.3 }
            )
        }
        @{
            Role = 'footsteps-wood'
            Folder = '2-obxjohn'
            Kind = 'walk'
            Title = 'Footsteps over Wooden Bridge'
            Page = 'https://freesound.org/people/OBXJohn/sounds/253090/'
            Author = 'OBXJohn'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'One person in boots crossing a real wooden bridge; recorded on a pocket voice recorder, so probably compressed once.'
            Files = @(
                @{ Name = '253090.wav'; FreesoundId = 253090; Sha256 = '803BD1DDA7D51482EAA64AFF0C8F6CA97664706DDBF65FC96706BCA64E9B1F2C'; Format = 'WAV'; SampleRate = 44100; Channels = 2; Seconds = 10.81; Above4k = 2.9; Edge99kHz = 9.5 }
            )
        }
        @{
            Role = 'footsteps-mud'
            Folder = '1-vrymaa'
            Kind = 'walk'
            Title = 'Footsteps Mud - Walk and run'
            Page = 'https://freesound.org/people/Vrymaa/sounds/770097/'
            Author = 'Vrymaa'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Close, dry and without background noise, says the author; walking first, then running.'
            Files = @(
                @{ Name = '770097.wav'; FreesoundId = 770097; Sha256 = '5FDFC8C60D4836DB2DAE197437901F599B27246C9BE63A245A486F5FB6170370'; Format = 'WAV'; SampleRate = 48000; Channels = 1; Seconds = 29.75; Above4k = 55.4; Edge99kHz = 13.9 }
            )
        }
        @{
            Role = 'footsteps-mud'
            Folder = '2-adielees9'
            Kind = 'walk'
            Title = 'Squelching Footsteps'
            Page = 'https://freesound.org/people/Adielees9/sounds/369579/'
            Author = 'Adielees9'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'A squelching walk, 96 kHz stereo.'
            Files = @(
                @{ Name = '369579.wav'; FreesoundId = 369579; Sha256 = '5D33DF5111DA93A7C852C718A23B568E537A8C8ABEFB543446DFF97F5066A856'; Format = 'WAV'; SampleRate = 96000; Channels = 2; Seconds = 11.50; Above4k = 51.5; Edge99kHz = 16.1 }
            )
        }
        @{
            Role = 'footsteps-mud'
            Folder = '3-nox-sound'
            Kind = 'walk'
            Title = 'Footsteps_Mountain_Boots_Mud_Mono'
            Page = 'https://freesound.org/people/Nox_Sound/sounds/548384/'
            Author = 'Nox_Sound'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Five walking steps in boots, then running and landings.'
            Files = @(
                @{ Name = '548384.wav'; FreesoundId = 548384; Sha256 = '3689BD78A28D59C5A38121DC2D6A85C2C68AD48E9A05A26AFED9152D829C22E7'; Format = 'WAV'; SampleRate = 48000; Channels = 1; Seconds = 14.64; Above4k = 40.6; Edge99kHz = 16.2 }
            )
        }
        @{
            Role = 'footsteps-water'
            Folder = '1-adielees9'
            Kind = 'walk'
            Title = 'Slow Water Footsteps'
            Page = 'https://freesound.org/people/Adielees9/sounds/369580/'
            Author = 'Adielees9'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Slow steps through a small still stream, 96 kHz stereo; the nearest thing to a ford.'
            Files = @(
                @{ Name = '369580.wav'; FreesoundId = 369580; Sha256 = '13D78029E37ED1C74CAF98F2790A2E7956D6F1765DFAB21B17118A10EF0D5120'; Format = 'WAV'; SampleRate = 96000; Channels = 2; Seconds = 12.80; Above4k = 58.4; Edge99kHz = 15.8 }
            )
        }
        @{
            Role = 'footsteps-water'
            Folder = '2-aglinder'
            Kind = 'walk'
            Title = 'Footsteps Water 01'
            Page = 'https://freesound.org/people/aglinder/sounds/265582/'
            Author = 'aglinder'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Walking and running in shallow water; the author mentions wind and a cicada, which may account for some of its brightness.'
            Files = @(
                @{ Name = '265582.wav'; FreesoundId = 265582; Sha256 = 'A9F0F9D43058A7886F46BF6217B3ECC0EAAF12ECCB289DF876BCE382BC2E992A'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 52.78; Above4k = 52.7; Edge99kHz = 17.1 }
            )
        }
        @{
            Role = 'footsteps-water'
            Folder = '3-christopherjngs'
            Kind = 'walk'
            Title = 'Splashing Footsteps Shallow Water'
            Page = 'https://freesound.org/people/ChristopherJngs/sounds/861369/'
            Author = 'ChristopherJngs'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Wading into and out of a puddle, mono.'
            Files = @(
                @{ Name = '861369.wav'; FreesoundId = 861369; Sha256 = '69A1B5C868E3491ACF349C6F5074A2DFE986C25CE32FF11FB293F179EFE03D01'; Format = 'WAV'; SampleRate = 44100; Channels = 1; Seconds = 8.87; Above4k = 42.1; Edge99kHz = 18.2 }
            )
        }
        @{
            Role = 'footsteps-snow'
            Folder = '1-xkeril'
            Kind = 'walk'
            Title = 'Footsteps on snow (clean)'
            Page = 'https://freesound.org/people/xkeril/sounds/611277/'
            Author = 'xkeril'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'A clean mono walk on snow; the only bright snow recording found.'
            Files = @(
                @{ Name = '611277.wav'; FreesoundId = 611277; Sha256 = 'BF01D790580EA0A400E6A0DFCD8425F6EF408CC8DE446D9A8DB77EA718145606'; Format = 'WAV'; SampleRate = 48000; Channels = 1; Seconds = 45.44; Above4k = 32.1; Edge99kHz = 15.5 }
            )
        }
        @{
            Role = 'footsteps-snow'
            Folder = '2-cabled-mess'
            Kind = 'steps'
            Title = 'Footstep in the snow 01 to 04, four single steps'
            Page = 'https://freesound.org/people/cabled_mess/sounds/384423/ (the other steps are in the same pack, https://freesound.org/people/cabled_mess/packs/21581/)'
            Author = 'cabled_mess'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Four unprocessed single steps, 96 kHz stereo; fairly dark.'
            Files = @(
                @{ Name = '384423.wav'; FreesoundId = 384423; Sha256 = 'EE5E0F8A850882FEE97BEAB8634E2DC455633702D08EB023A2ED0361B43EEA73'; Format = 'WAV'; SampleRate = 96000; Channels = 2; Seconds = 1.17; Above4k = 17.1; Edge99kHz = 15.0 }
                @{ Name = '384422.wav'; FreesoundId = 384422; Sha256 = 'A245A967D34FFA9EDC1E971A72B53AFD3742E4606F586BCB4C110B75986DC53D'; Format = 'WAV'; SampleRate = 96000; Channels = 2; Seconds = 1.09; Above4k = 9.9; Edge99kHz = 12.3 }
                @{ Name = '384421.wav'; FreesoundId = 384421; Sha256 = '076688529A0DD996BEA9E4760C3F7D628DE06E1715167BE3EA000C22A9CD440F'; Format = 'WAV'; SampleRate = 96000; Channels = 2; Seconds = 1.76; Above4k = 9.6; Edge99kHz = 12.7 }
                @{ Name = '384424.wav'; FreesoundId = 384424; Sha256 = '002062FCCB49F96977EFB2ABDCB50B3AB243F8250CF0EA07DBB43ED37B391450'; Format = 'WAV'; SampleRate = 96000; Channels = 2; Seconds = 0.97; Above4k = 4.3; Edge99kHz = 10.5 }
            )
        }
        @{
            Role = 'ambience-meadow'
            Folder = '1-dangasior'
            Kind = 'bed'
            Title = 'Summer afternoon Meadow, birds and crickets'
            Page = 'https://freesound.org/people/DanGasior/sounds/414907/'
            Author = 'DanGasior'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Birds and crickets, recorded with a SoundField microphone; nothing else mentioned.'
            Files = @(
                @{ Name = '414907.wav'; FreesoundId = 414907; Sha256 = 'E19FE5985F857AF0A202FCE04389B37F910EDE8ADD7A7C9BFE98D329DC334ED2'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 121.26; Above4k = 46.4; Edge99kHz = 22.0 }
            )
        }
        @{
            Role = 'ambience-meadow'
            Folder = '2-richwise'
            Kind = 'bed'
            Title = 'Summer meadow - Houghton Lodge'
            Page = 'https://freesound.org/people/richwise/sounds/815552/'
            Author = 'richwise'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Insects in long grass and an occasional bird; two pigeons fly over, and a breeze touches the microphone after about a minute.'
            Files = @(
                @{ Name = '815552.wav'; FreesoundId = 815552; Sha256 = '4713F635549B89A51C68B8DD31171D5332B607010FC88727AA206A29B3207383'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 113.82; Above4k = 67.0; Edge99kHz = 23.5 }
            )
        }
        @{
            Role = 'ambience-meadow'
            Folder = '3-alcappuccino'
            Kind = 'bed'
            Title = 'Hungary Meadow Morning Birds'
            Page = 'https://freesound.org/people/Alcappuccino/sounds/416205/'
            Author = 'Alcappuccino'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Morning birds and crickets, hand-held recorder; darker.'
            Files = @(
                @{ Name = '416205.wav'; FreesoundId = 416205; Sha256 = '0DB4BBAF76EF71D13E5A702E2385131C0DF77414B52E2EC60CEED4637276F468'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 211.04; Above4k = 3.9; Edge99kHz = 4.6 }
            )
        }
        @{
            Role = 'ambience-forest'
            Folder = '1-fester993'
            Kind = 'bed'
            Title = 'Forest Ambience 1'
            Page = 'https://freesound.org/people/Fester993/sounds/564436/'
            Author = 'Fester993'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'A plain summer forest in Italy; the best-rated forest bed on Freesound.'
            Files = @(
                @{ Name = '564436.wav'; FreesoundId = 564436; Sha256 = 'BE406806ABC98EBF420CB23434FB6B6E718C0879DD474F335B9C6D195F99B38B'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 181.18; Above4k = 14.8; Edge99kHz = 20.0 }
            )
        }
        @{
            Role = 'ambience-forest'
            Folder = '2-nfsgit'
            Kind = 'bed'
            Title = 'Quiet forest, birds, insects, leaf rustle'
            Page = 'https://freesound.org/people/nfsgit/sounds/121965/'
            Author = 'nfsgit'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Deep in a forest north of Oslo, birds and insects; has low rumble.'
            Files = @(
                @{ Name = '121965.wav'; FreesoundId = 121965; Sha256 = '4A8CB9C0430B2EE2CD9417C26F2339738F917974F0861848D281ECF5ACDDC34E'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 292.25; Above4k = 19.2; Edge99kHz = 15.0 }
            )
        }
        @{
            Role = 'ambience-forest'
            Folder = '3-burghrecords'
            Kind = 'bed'
            Title = 'Birds In Forest (Scotland)'
            Page = 'https://freesound.org/people/BurghRecords/sounds/490846/'
            Author = 'BurghRecords'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Many birds and wind in the trees; may sound too much like the meadow.'
            Files = @(
                @{ Name = '490846.wav'; FreesoundId = 490846; Sha256 = '72CEA658FC77666EB5464AD065C40CB1BBCE0132BBB91037590203875C35F7C8'; Format = 'WAV'; SampleRate = 44100; Channels = 2; Seconds = 113.02; Above4k = 65.4; Edge99kHz = 7.8 }
            )
        }
        @{
            Role = 'ambience-dense-forest'
            Folder = '1-greysound'
            Kind = 'bed'
            Title = 'Clark''s Tower Ambience 4'
            Page = 'https://freesound.org/people/greysound/sounds/547922/'
            Author = 'greysound'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Strong wind through the trees, creaking trunks, distant birds; cleaned of wind pops.'
            Files = @(
                @{ Name = '547922.flac'; FreesoundId = 547922; Sha256 = '5AEFB03EFFF79D84690F5A97A842F4345360B10488D681C6114455F533F03EF7'; Format = 'FLAC'; SampleRate = 48000; Channels = 2; Seconds = 460.00; Above4k = 7.3; Edge99kHz = 8.9 }
            )
        }
        @{
            Role = 'ambience-dense-forest'
            Folder = '2-bruno-auzet'
            Kind = 'bed'
            Title = 'Silent windy pine forest'
            Page = 'https://freesound.org/people/bruno.auzet/sounds/670307/'
            Author = 'bruno.auzet'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Wind in tall pines, a trunk grinding now and then; very dark, may sound like the mountain.'
            Files = @(
                @{ Name = '670307.wav'; FreesoundId = 670307; Sha256 = '2A369FAAB1F3E8DD3D896EDEA89D554F776E44C5B98A33C08D08A7C8E406DBF5'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 334.11; Above4k = 0.7; Edge99kHz = 3.3 }
            )
        }
        @{
            Role = 'ambience-dense-forest'
            Folder = '3-dokuta-gerovv'
            Kind = 'bed'
            Title = 'Forest at night'
            Page = 'https://freesound.org/people/Dokuta_Gerovv/sounds/662095/'
            Author = 'Dokuta_Gerovv'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'A summer night in a Greek forest near a village; insects may sound like the marsh.'
            Files = @(
                @{ Name = '662095.wav'; FreesoundId = 662095; Sha256 = 'D70E57D0EE92266992D0CEC98580BFF4DBB4F7951A35D6310ADE5B89CDEBE5FB'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 77.99; Above4k = 23.4; Edge99kHz = 11.5 }
            )
        }
        @{
            Role = 'ambience-mountain'
            Folder = '1-felix-blume'
            Kind = 'bed'
            Title = 'Wind singing in the mountain'
            Page = 'https://freesound.org/people/felix.blume/sounds/165526/'
            Author = 'felix.blume'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Wind howling on a mountaintop in Mexico, crickets behind; the howl gives it a clear identity.'
            Files = @(
                @{ Name = '165526.wav'; FreesoundId = 165526; Sha256 = 'DB51FC815465C19CCEBA2A2E37ED03874F1BAEB945FFB3680C5D56074B107A45'; Format = 'WAV'; SampleRate = 96000; Channels = 2; Seconds = 230.10; Above4k = 0.5; Edge99kHz = 1.7 }
            )
        }
        @{
            Role = 'ambience-mountain'
            Folder = '2-kyles'
            Kind = 'bed'
            Title = 'Mountain wind, heavy strong gusts, Swiss Alps'
            Page = 'https://freesound.org/people/kyles/sounds/454092/'
            Author = 'kyles'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Heavy gusts; a loop may show its seam.'
            Files = @(
                @{ Name = '454092.flac'; FreesoundId = 454092; Sha256 = '2D9CD117FC3B350AD13E16BD6C84C460367D7BACBBB94006A4DB6A0704C794FF'; Format = 'FLAC'; SampleRate = 48000; Channels = 2; Seconds = 171.22; Above4k = 1.2; Edge99kHz = 4.3 }
            )
        }
        @{
            Role = 'ambience-mountain'
            Folder = '3-nicotep'
            Kind = 'bed'
            Title = 'Larrun Mountains, medium wind'
            Page = 'https://freesound.org/people/nicotep/sounds/725630/'
            Author = 'nicotep'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Steady moderate wind, Basque Country; the least distinctive.'
            Files = @(
                @{ Name = '725630.aiff'; FreesoundId = 725630; Sha256 = '7FFD9A4D052D4431D746ED68C2EB912B18418E2DBAC919B35E068A615DA1C4D2'; Format = 'AIFF'; SampleRate = 48000; Channels = 2; Seconds = 150.85; Above4k = 0.5; Edge99kHz = 2.6 }
            )
        }
        @{
            Role = 'ambience-marsh'
            Folder = '1-danjocross'
            Kind = 'bed'
            Title = 'Lily pads and frogs at night'
            Page = 'https://freesound.org/people/Danjocross/sounds/503206/'
            Author = 'Danjocross'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Loud frogs and katydids at a lake at night; the author says unedited, with no car or plane noise.'
            Files = @(
                @{ Name = '503206.wav'; FreesoundId = 503206; Sha256 = '9F025D155569C02CDE3334DCA8F3858B020D77A00988AABC54B06898C03BE159'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 111.87; Above4k = 83.2; Edge99kHz = 13.8 }
            )
        }
        @{
            Role = 'ambience-marsh'
            Folder = '2-marleneayni'
            Kind = 'bed'
            Title = 'Marsh'
            Page = 'https://freesound.org/people/MarleneAyni/sounds/580325/'
            Author = 'MarleneAyni'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Green frogs and many birds by day, Quebec; has wind rumble.'
            Files = @(
                @{ Name = '580325.wav'; FreesoundId = 580325; Sha256 = 'AE5DAFADC8132B66EF48C8668AB6617DFD7FB8E8D6FEA578CD0264066CB13412'; Format = 'WAV'; SampleRate = 44100; Channels = 2; Seconds = 154.37; Above4k = 2.4; Edge99kHz = 7.8 }
            )
        }
        @{
            Role = 'ambience-marsh'
            Folder = '3-jacekksiazek'
            Kind = 'bed'
            Title = 'Swamp, frogs, birds, forest'
            Page = 'https://freesound.org/people/jacekksiazek/sounds/393644/'
            Author = 'jacekksiazek'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Frogs and birds in a Polish swamp reserve; dark.'
            Files = @(
                @{ Name = '393644.wav'; FreesoundId = 393644; Sha256 = '9FBABEB485D7D58C6B6CD5F289D4C9C4763C580105E599046EBF5AD7913319C2'; Format = 'WAV'; SampleRate = 96000; Channels = 2; Seconds = 215.89; Above4k = 0.7; Edge99kHz = 3.5 }
            )
        }
        @{
            Role = 'ambience-river'
            Folder = '1-klau78'
            Kind = 'bed'
            Title = 'Small river stream'
            Page = 'https://freesound.org/people/klau78/sounds/546804/'
            Author = 'klau78'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'A small river in Transylvania, continuous flow, nothing else mentioned.'
            Files = @(
                @{ Name = '546804.wav'; FreesoundId = 546804; Sha256 = '52845FC98FFF7BF0A0CC558205ACA26F927E5BE5006ECAFC5B6C5232610D972D'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 340.91; Above4k = 1.8; Edge99kHz = 4.7 }
            )
        }
        @{
            Role = 'ambience-river'
            Folder = '2-extemporalist'
            Kind = 'bed'
            Title = 'Snowmelt flowing into a lake, Okanagan'
            Page = 'https://commons.wikimedia.org/wiki/File:Snowmelt_flowing_into_lake_at_Okanagan_Mountain_Provincial_Park.flac'
            Author = 'Extemporalist'
            Licence = 'This file is made available under the Creative Commons CC0 1.0 Universal Public Domain Dedication.'
            LicenceChecked = 'Wikimedia Commons API: licence CC0, page templates self|cc-zero'
            Notes = 'Fresh water flowing; 33 minutes long, the best Commons find.'
            Files = @(
                @{ Name = 'okanagan-snowmelt.flac'; Url = 'https://upload.wikimedia.org/wikipedia/commons/8/8d/Snowmelt_flowing_into_lake_at_Okanagan_Mountain_Provincial_Park.flac'; Sha256 = 'B38162DA639293226A87830E9485E42D99B8947F35CDC2DF152C01A4D064DF65'; Format = 'FLAC'; SampleRate = 48000; Channels = 2; Seconds = 1960.12; Above4k = 13.2; Edge99kHz = 12.5 }
            )
        }
        @{
            Role = 'ambience-river'
            Folder = '3-drtrumpet'
            Kind = 'bed'
            Title = 'Molln stream, Bachlauf'
            Page = 'https://commons.wikimedia.org/wiki/File:2024-07-26_Molln_(Ober%C3%B6sterreich)_Bachlauf_pl%C3%A4tschert_(Krumme_Steyerling_bei_Piesslingerstra%C3%9Fe).wav'
            Author = 'DrTrumpet'
            Licence = 'This file is made available under the Creative Commons CC0 1.0 Universal Public Domain Dedication.'
            LicenceChecked = 'Wikimedia Commons API: licence CC0, page templates self|cc-zero'
            Notes = 'A babbling stream, close, on a studio microphone; mono, 27 seconds.'
            Files = @(
                @{ Name = 'molln-bachlauf.wav'; Url = 'https://upload.wikimedia.org/wikipedia/commons/2/29/2024-07-26_Molln_%28Ober%C3%B6sterreich%29_Bachlauf_pl%C3%A4tschert_%28Krumme_Steyerling_bei_Piesslingerstra%C3%9Fe%29.wav'; Sha256 = '862D5754748E346EE127A0B0415F1AE8C92F57A516922896B51FC36F24F77F5F'; Format = 'WAV'; SampleRate = 48000; Channels = 1; Seconds = 26.58; Above4k = 13.9; Edge99kHz = 12.6 }
            )
        }
        @{
            Role = 'ambience-lake'
            Folder = '1-trp'
            Kind = 'bed'
            Title = 'Water waves lapping, lake, gentle, close'
            Page = 'https://freesound.org/people/TRP/sounds/568819/'
            Author = 'TRP'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Small waves lapping a rocky shore, recorded close; clearly not the sea.'
            Files = @(
                @{ Name = '568819.flac'; FreesoundId = 568819; Sha256 = '9A2D3FD1871F9616BEAB1B5591AA505F47A44CA2D14C542EDA956D83DAB854A0'; Format = 'FLAC'; SampleRate = 48000; Channels = 2; Seconds = 132.73; Above4k = 5.3; Edge99kHz = 11.2 }
            )
        }
        @{
            Role = 'ambience-lake'
            Folder = '2-theflyfishingfilmmaker'
            Kind = 'bed'
            Title = 'Gentle waves on a lake'
            Page = 'https://freesound.org/people/TheFlyFishingFilmmaker/sounds/614299/'
            Author = 'TheFlyFishingFilmmaker'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Gentle waves on rocks; has wind rumble.'
            Files = @(
                @{ Name = '614299.wav'; FreesoundId = 614299; Sha256 = 'F70458EEFA21A673270187EA488A2215602FCA1A8CC76CFEDF1F7D2042484EB0'; Format = 'WAV'; SampleRate = 44100; Channels = 2; Seconds = 175.47; Above4k = 4.7; Edge99kHz = 8.7 }
            )
        }
        @{
            Role = 'ambience-sea'
            Folder = '1-yevgverh'
            Kind = 'bed'
            Title = 'Ocean coast, Outer Banks'
            Page = 'https://freesound.org/people/YevgVerh/sounds/827530/'
            Author = 'YevgVerh'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Light ocean waves on a sand beach.'
            Files = @(
                @{ Name = '827530.wav'; FreesoundId = 827530; Sha256 = '1F3A8241996F43BAFC66F581079CCEF92FF0E8CE3FA698A988522685479C29ED'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 181.18; Above4k = 1.5; Edge99kHz = 5.5 }
            )
        }
        @{
            Role = 'ambience-sea'
            Folder = '2-molkko'
            Kind = 'bed'
            Title = 'Sea waves'
            Page = 'https://freesound.org/people/molkko/sounds/425632/'
            Author = 'molkko'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Mid-sized Mediterranean waves; the brightest sea.'
            Files = @(
                @{ Name = '425632.wav'; FreesoundId = 425632; Sha256 = '27F0E02069A36462ACEC6A8F6D6CC6E9E99BBD28EE2A0E0E97404305A67EFEE3'; Format = 'WAV'; SampleRate = 44100; Channels = 2; Seconds = 122.93; Above4k = 4.6; Edge99kHz = 11.2 }
            )
        }
        @{
            Role = 'ambience-sea'
            Folder = '3-huhuertas'
            Kind = 'bed'
            Title = 'Sea waves, Playas del Viento'
            Page = 'https://freesound.org/people/huhuertas/sounds/786037/'
            Author = 'huhuertas'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'Sea waves on a Colombian beach.'
            Files = @(
                @{ Name = '786037.wav'; FreesoundId = 786037; Sha256 = '5B2BC8317B191B6D88163D7E6E8D8EAE369B00DFBBF72A6DE0AA8030BB40E06E'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 207.72; Above4k = 1.5; Edge99kHz = 5.3 }
            )
        }
        @{
            Role = 'ambience-town'
            Folder = '1-ninebilly'
            Kind = 'bed'
            Title = 'Market'
            Page = 'https://freesound.org/people/ninebilly/sounds/173013/'
            Author = 'ninebilly'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'A market crowd; place and language unknown, so speech may be clear.'
            Files = @(
                @{ Name = '173013.wav'; FreesoundId = 173013; Sha256 = 'DBE08C57CD28A62930767694C5E39843887CD6F69CFF27B42BC973C48A70D071'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 178.15; Above4k = 1.4; Edge99kHz = 4.9 }
            )
        }
        @{
            Role = 'ambience-town'
            Folder = '2-aldor'
            Kind = 'bed'
            Title = 'La fontaine de la place Saint-Etienne'
            Page = 'https://commons.wikimedia.org/wiki/File:La_fontaine_de_la_place.ogg'
            Author = 'aldor'
            Licence = 'This work has been released into the public domain by its author, aldor.'
            LicenceChecked = 'Wikimedia Commons API: licence Public domain, page templates PD-pdsounds.org, PD-author'
            Notes = 'A town fountain with cathedral bells, Toulouse; came from pdsounds.org as a 128 kbps MP3.'
            Files = @(
                @{ Name = 'toulouse-fountain.ogg'; Url = 'https://upload.wikimedia.org/wikipedia/commons/6/6f/La_fontaine_de_la_place.ogg'; Sha256 = '6E1C33FC1BD370608D17BF3A888DC4D28F19A4687A3E81AD185F759B063EC82A'; Format = 'OGG'; SampleRate = 44100; Channels = 2; Seconds = 85.00; Above4k = 14.2; Edge99kHz = 14.2 }
            )
        }
        @{
            Role = 'ambience-town'
            Folder = '3-samuelgremaud'
            Kind = 'bed'
            Title = 'Crowd'
            Page = 'https://freesound.org/people/SamuelGremaud/sounds/545756/'
            Author = 'SamuelGremaud'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'A market crowd in the south of France; expect French speech.'
            Files = @(
                @{ Name = '545756.wav'; FreesoundId = 545756; Sha256 = 'E46758646D008C23109657E18740A1C99B744422A697CE5E526FA010CD266862'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 167.27; Above4k = 0.6; Edge99kHz = 3.3 }
            )
        }
        @{
            Role = 'ambience-town'
            Folder = '4-bruno-auzet'
            Kind = 'bed'
            Title = 'Traditional blacksmith'
            Page = 'https://freesound.org/people/bruno.auzet/sounds/640197/'
            Author = 'bruno.auzet'
            Licence = 'http://creativecommons.org/publicdomain/zero/1.0/ (the licence field Freesound gives for this sound through its API)'
            LicenceChecked = 'Freesound API, licence field read for every file'
            Notes = 'A forge with fire and gentle hammering; not a bed alone, but could sit under a crowd.'
            Files = @(
                @{ Name = '640197.wav'; FreesoundId = 640197; Sha256 = '3313AC4DBA095AEF04BFCFCDC571C36EE330C6E0F4A96732E7F1D01BAF648722'; Format = 'WAV'; SampleRate = 48000; Channels = 2; Seconds = 311.83; Above4k = 32.3; Edge99kHz = 15.6 }
            )
        }
    )
}
