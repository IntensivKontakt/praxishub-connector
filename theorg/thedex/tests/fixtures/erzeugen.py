"""Erzeugt die THEDEX-Testdateien (Win-1252 bzw. CP850, CRLF)."""
import os
import sys

ziel = sys.argv[1]
os.makedirs(ziel, exist_ok=True)


def schreiben(name, text, enc="cp1252", eol="\r\n"):
    text = text.strip("\n") + "\n"
    daten = text.replace("\n", eol).encode(enc)
    with open(os.path.join(ziel, name), "wb") as f:
        f.write(daten)


KOPF_THEORG = """[Thedex]
ThedexVersionNumber=2.1

[Sender]
SenderIdentification=THEORG
SenderVerification=02A7
SenderDescription=THEORG Praxis-Management
SenderSoftwareVersionInformation=11.58
SenderLanIdentification=THEORG
"""

# Spezifikation 2.7, Nachricht vom Typ "ClientNew" (S. 19-20), wörtlich.
schreiben("SPECNEW.TDX", KOPF_THEORG + """
[Message]
MessageType=ClientNew
MessageCountryInformation=049
MessageCodePageInformation=ANSI
MessageDate=11092014
MessageTime=09:52

[Client]
CSenderInternalIdentification=1166
CExternalIdentification=
CTitle=
CLastName=Mustermann
CFirstName=Julia
CBirthday=11011965
CSex=F
CHeight=
CWeight=
CStreet=Musterstr. 11
CZip=99999
CTown=Musterhausen
CCountryCode=
CCountry=
CTelephon=
CFax=
CCheckInID=200031969
InsuranceID=108018347
InsuranceClientID=1234567890
InsuranceStatus=1
InsuranceDebitnumber=88562347
ClinicDepartment=REHA/1
ClinicRoomNumber=205
ClinicCaseNumber=
CustomerAccount=90158
eMail=Julia.Mustermann@t-online.de
CellularPhone=+49172875569
CBankIdendificationCode=10010000
CBankName=BBk Berlin
CAccountNumber=10203040
CAlternativeAccountHolder=Hans Mustermann
CBIC=BSBNDEFF
CIBAN=DE18100100000010203040
UserDefined=6
UserDefined1=
UserDefined2=Schmerzpatient
UserDefined3=
UserDefined4=
UserDefined5=
UserDefined6=
""")

# Spezifikation 2.7, Nachricht vom Typ "ClientChange" (S. 20-22), wörtlich.
schreiben("SPECCHG.TDX", KOPF_THEORG + """
[Message]
MessageType=ClientChange
MessageCountryInformation=049
MessageCodePageInformation=ANSI
MessageDate=11092014
MessageTime=09:52

[Client]
CSenderInternalIdentification=1166
CExternalIdentification=
CTitle=
CLastName=Mustermann
CFirstName=Julia
CBirthday=11011965
CSex=F
CHeight=
CWeight=
CStreet=Musterstr. 11
CZip=99999
CTown=Musterhausen
CCountryCode=
CCountry=
CTelephon=
CFax=011/98798745
CCheckInID=200031969
InsuranceID=
InsuranceClientID=
InsuranceStatus=
InsuranceDebitnumber=
ClinicDepartment=REHA/1
ClinicRoomNumber=205
ClinicCaseNumber=
CustomerAccount=701546
eMail=Julia.mustermann@t-online.de
CellularPhone=+491725556789
CBankIdendificationCode=10010000
CBankName=BBk Berlin
CAccountNumber=10203040
CAlternativeAccountHolder=Hans Mustermann
CBIC=BSBNDEFF
CIBAN=DE18100100000010203040
UserDefined=6
UserDefined1=
UserDefined2=Herzschwäche
UserDefined3=
UserDefined4=
UserDefined5=
UserDefined6=

[ClientNewData]
CSenderInternalIdentification=1166
CExternalIdentification=
CTitle=
CLastName=Mustermann
CFirstName=Julia
CBirthday=11011965
CSex=F
CHeight=169
CWeight=62.7
CStreet=Marktplatz 17
CZip=99998
CTown=Musterdorf
CCountryCode=D
CCountry=
CTelephon=09999/1234567
CFax=011/98798745
CCheckInID=200031969
InsuranceID=108018347
InsuranceClientID=1234567890
InsuranceStatus=1
InsuranceDebitnumber=88562347
ClinicDepartment=REHA/2
ClinicRoomNumber=319
ClinicCaseNumber=
CustomerAccount=701546
eMail=Julia.mustermann@t-online.de
CellularPhone=+491725556789
CBankIdendificationCode=10010000
CBankName=BBk Berlin
CAccountNumber=10203040
CAlternativeAccountHolder=Hans Mustermann
CBIC=BSBNDEFF
CIBAN=DE18100100000010203040
UserDefined=6
UserDefined1=
UserDefined2=Herzschwäche
UserDefined3=
UserDefined4=
UserDefined5=
UserDefined6=
""")

ERIKA = """[Client]
CSenderInternalIdentification=339
CExternalIdentification=51-781451-78
CTitle=Prof. Dr. med.
CLastName=Mustermann
CFirstName=Erika
CBirthday=17031959
CSex=W
CHeight=178
CWeight=76.4
CStreet=Musterstrasse 17
CZip=71636
CTown=Ludwigsburg
CCountryCode=D
CCountry=Deutschland
CTelephone=01234/12345
CFax=01234/23456
CCheckInID=200031969
InsuranceID=108018347
InsuranceClientID=1234567890
InsuranceStatus=1
InsuranceDebitnumber=88562347
ClinicDepartment=REHA/1
ClinicRoomNumber=205
ClinicCaseNumber=478FH/201107108
CustomerAccount=90156
eMail=erika@mustermann.de
CellularPhone=+491725556789
CBankIdendificationCode=10010000
CBankName=BBk Berlin
CAccountNumber=10203040
CAlternativeAccountHolder=Hans Mustermann
CBIC=BSBNDEFF
CIBAN=DE18100100000010203040
"""

# Spezifikation 2.7, Nachricht vom Typ "ClientReport" (S. 22-23), wörtlich
# (inkl. Tippfehler "SenderLanIdetifikation" und Leerzeichen in UserDefined1).
schreiben("SPECREP.TDX", """
[Thedex]
ThedexVersionNumber=2.1

[Sender]
SenderIdentification=mechaTronic
SenderVerification=038D
SenderDescription=DigiMax Meß- und Auswertesoftware
SenderSoftwareVersionInformation=7.74
SenderLanIdetifikation=Deutschland

[Message]
MessageType=ClientReport
MessageCountryInformation=049
MessageCodePageInformation=ANSI
MessageDate=11092014
MessageTime=16:10

""" + ERIKA + """UserDefined=4
UserDefined1= Rückenschule
UserDefined2=
UserDefined3=
UserDefined4=

[ReportData]
ReportDate=11092014
ReportTime=09:17
ReportDescription=Neue Daten
ReportTherapist=
ReportType=BMP
ReportFile=00010001.bmp
""")

# Synthetischer Anhang zum ClientReport (nur BMP-Kennung, kein echtes Bild).
with open(os.path.join(ziel, "00010001.bmp"), "wb") as f:
    f.write(b"BM" + bytes(52))

# Abschnittsbeispiele aus 2.5 zu einer ClientCheckInOut-Nachricht von THEORG
# zusammengesetzt: [Thedex] S. 12, [Sender] S. 13, [Client] S. 16-17,
# [CheckInOutData] S. 19. MessageType angepasst.
schreiben("SPECCIO.TDX", """
[Thedex]
ThedexVersionNumber=2.1

[Sender]
SenderIdentification=THEORG
SenderVerification=02A7
SenderDescription=Theorg Praxis Management
SenderSoftwareVersionInformation=11.42
SenderLanIdentification=THEORG

[Message]
MessageType=ClientCheckInOut
MessageCountryInformation=049
MessageCodePageInformation=ANSI
MessageDate=11092014
MessageTime=20:48

""" + ERIKA + """
[CheckInOutData]
Event=OUT
CheckInDate=11092014
CheckInTime=17:17
CheckOutDate=11092014
CheckOutTime=20:48
Description=Vertrag 36 Monate
""")

# [Message]-Beispiel S. 13 und [ReportData]-Beispiel S. 18 (Anhang fehlt absichtlich).
schreiben("SPECRDA.TDX", KOPF_THEORG + """
[Message]
MessageType=ClientReport
MessageCountryInformation=049
MessageCodePageInformation=ANSI
MessageDate=11092014
MessageTime=16:10

[Client]
CSenderInternalIdentification=339
CLastName=Mustermann
CFirstName=Erika
CBirthday=17031959

[ReportData]
ReportDate=11092014
ReportTime=09:17
ReportDescription=Trainingsbericht ErgoBike
ReportTherapist=Heike
ReportType=BMP
ReportFile=REP00001.BMP
""")

# Bilaterale Abschnitte [Cybex] S. 25 und [mechaTronic] S. 26 sowie das
# UserDefined-Beispiel S. 19 in einer ClientInfo von THEORG.
schreiben("SPECBIL.TDX", KOPF_THEORG + """
[Message]
MessageType=ClientInfo
MessageCountryInformation=049
MessageCodePageInformation=ANSI
MessageDate=11092014
MessageTime=10:00

[Client]
CSenderInternalIdentification=933
CLastName=Mustermann
CFirstName=Max
CBirthday=01021970
CSex=M
UserDefined=6
UserDefined1=Patient hat starke Schmerzen
UserDefined2=Hautallergie
UserDefined3=
UserDefined4=
UserDefined5=Sportler
UserDefined6=

[Cybex]
PatientID=933 TH
SSNO=1122334455
Height=175
Domhand=left
Injurycode=sport
Referphys=Dr. med. Klaus Hoppe
Diagnosis=Kreuzbandruptur
Insurance=AOK Stuttgart
Text1=
Text2=
Text3=
Num1=
Num2=
Num3=
Notes=
MI=
BodyWeight=82
ClinicanID=

[mechaTronic]
PatientID=933 TH
Height=175
Insurance=AOK Stuttgart
BodyWeight=82
ClinicanID=
""")

# Synthetisch: Mitgliedsvertrag (die Spezifikation hat kein Beispiel für [ContractData]).
schreiben("VERTRAG.TDX", KOPF_THEORG + """
[Message]
MessageType=ClientInfo
MessageCountryInformation=049
MessageCodePageInformation=ANSI
MessageDate=15012015
MessageTime=08:30

[Client]
CSenderInternalIdentification=2001
CLastName=Beispiel
CFirstName=Jörg
CBirthday=29021984
CSex=M
CCheckInID=CHIP-0815
InsuranceID=PRIVAT
InsuranceStatus=P

[ContractData]
BeginDate=01012014
EndDate=31122015
TerminationDate=30092015
SuspendBeginDate=01072014
SuspendEndDate=31072014
Description=Gerätetraining 24 Monate
""")

# Synthetisch: zwei [ContractData]-Abschnitte (ob THEORG das tut, ist offen).
schreiben("VERTRAG2.TDX", KOPF_THEORG + """
[Message]
MessageType=ClientInfo
MessageCodePageInformation=ANSI
MessageDate=15012015
MessageTime=08:31

[Client]
CSenderInternalIdentification=2002
CLastName=Probe
CFirstName=Anna

[ContractData]
BeginDate=01012014
Description=Rückenkurs

[ContractData]
BeginDate=01022014
Description=Gerätetraining

[ContractData]
BeginDate=
EndDate=
Description=
""")

# Synthetisch: OEM (CP850) mit Umlauten.
schreiben("OEMNEW.TDX", KOPF_THEORG + """
[Message]
MessageType=ClientNew
MessageCountryInformation=049
MessageCodePageInformation=OEM
MessageDate=02032015
MessageTime=11:05

[Client]
CSenderInternalIdentification=3003
CLastName=Müßig
CFirstName=Jürgen
CBirthday=05061970
CSex=M
CStreet=Hauptstraße 5
CZip=50667
CTown=Köln
""", enc="cp850")

# Synthetisch: Löschung.
schreiben("DELETE.TDX", KOPF_THEORG + """
[Message]
MessageType=ClientDelete
MessageCodePageInformation=ANSI
MessageDate=03032015
MessageTime=12:00

[Client]
CSenderInternalIdentification=1166
CLastName=Mustermann
CFirstName=Julia
CBirthday=11011965
""")

# Synthetisch: Grenzfälle der Syntax (LF, Kleinschreibung, Leerraum,
# unbekannte Felder/Abschnitte, leere Felder, kein Zeichensatz angegeben).
schreiben("UNBEK.TDX", """
; Kommentar vor dem ersten Abschnitt
[THEDEX]
thedexversionnumber = 2.1
ThedexBuild=7

[sender]
senderidentification=theorg
senderverification=02A7

[MESSAGE]
messagetype=clientinfo
MessageDate=04032015
MessageTime=7:05
MessageExtra=x

[client]
csenderinternalidentification = 4004
clastname = Leerfeld
cfirstname=
cbirthday=
csex=
ctelephone=  030 1234567
CFoo=Bär
UserDefined3=nur drei

[THEORG]
Zusatz=unbelegt
""", eol="\n")

schreiben("NOMSG.TDX", KOPF_THEORG + """
[Client]
CSenderInternalIdentification=5005
CLastName=Ohne
""")

schreiben("NOCLIENT.TDX", KOPF_THEORG + """
[Message]
MessageType=ClientInfo
MessageDate=04032015
""")

schreiben("NOKOPF.TDX", """
[Message]
MessageType=ClientInfo
MessageDate=04032015
MessageTime=09:00

[Client]
CSenderInternalIdentification=6006
CLastName=Kopflos
""")

with open(os.path.join(ziel, "KAPUTT.TDX"), "wb") as f:
    f.write(b"\x00\x01\x02PK\x03\x04\xff\xfe binary \x00\x00")

# Spezifikation 5.1, Beispiel THEDEX.INI (S. 28).
schreiben("THEDEX.INI", """
[THEDEX-Providers]
THEORG=Theorg-Praxis-Management
MECHTRON=mechaTronic DigiMax Meß- und Auswertesoftware
CYBEX=Cybex
BITSOFT=Fitness-Test Professionell

[THEORG_PROXOS]
Export=1
CPhoto=1
""")
